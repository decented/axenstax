# Skin painter — intuitive controls

**Date:** 2026-07-25
**Status:** BUILT 2026-07-25 (v0.2.16); extended 2026-09-06 with tools (fill/shade/noise/line), recent colours, hex entry — see cosmetics.md
**Supersedes nothing.** Extends `2026-06-29-skin-studio-wardrobe-design.md` and the
in-Workshop painter delivered by `plans/2026-06-30-skin-paint-in-workshop.md`.

## Why

Axolittle hit resistance painting his avatar and described it as "the limbs aren't
intuitive to turn on and off so you can paint each body part", plus the outer skin
not being intuitive. Investigation says he is not confused — he hit a wall.

### Finding 1: ~30% of the skin cannot be painted at all

The avatar's six boxes sit flush against one another (`skin_hit.rs:21-28`) and the
painter ray-casts them from outside, taking the nearest box entered from outside
(`ray_hit_avatar`). Boxes that touch therefore occlude each other permanently.

Measured by porting the geometry and running a per-face escape test (every sample
point on every face, against a hemisphere of escape directions, counting a point
buried inside a neighbouring box as blocked):

| Face | Paintable today |
|---|---|
| `body` +Y (top) | **0%** — head sits on it |
| `body` −Y (bottom) | **0%** — legs sit under it |
| `arm_L` +X, `arm_R` −X (inner arms) | **0%** — torso |
| `leg_L` +X, `leg_R` −X (inner legs) | **0%** — each other |
| `leg_L` +Y, `leg_R` +Y (leg tops) | **0%** — torso |
| `body` ±X (sides) | 10% — a sliver below the armpit |
| `head` −Y (bottom) | 40% |
| `head` ±X | 88% |

**8 of 36 faces are fully unreachable; overall paintable surface is 70.4%.**
Axo's instinct — separate the limbs — is the correct fix, and no such control exists.

### Finding 2: the Workshop hands you no dyes

`chunk_stream.rs:469-474` puts a Bellows in slot 9 and nothing else. Painting
requires "hold a dye", so the first-run path is: blow up the mannequin → left-click →
nothing happens → deduce that you must open the inventory and search for dye. The
hotbar has 9 slots for 16 dye colours. The `O` colour picker added 2026-07-05 widens
the colour range but holds one colour at a time and is absent from the player guide.

### Finding 3: you can paint pixels you cannot see

The mannequin always draws base + overlay together (`entity_model.rs:1387`), but the
Base ray-cast ignores the overlay shell entirely. On a skin with an opaque jacket,
clicks pass through it and paint the body underneath, invisibly. `V` announces the
layer switch with a 2-second toast and nothing persistent.

### Finding 4: the overlay offset is wrong, and reads as zero

`OVERLAY_INFLATE` is a flat `0.03` for every part (`entity_model.rs:21`, duplicated
at `skin_hit.rs:31`). Minecraft inflates per part, and the amount differs:

| Part | Minecraft grows the box | Per side | Blocks |
|---|---|---|---|
| Head (hat) | 1 px total | 0.5 px | 0.03125 |
| Body, arms, legs | 0.5 px total | 0.25 px | 0.015625 |

(The wiki states total growth; the model source states the per-side `CubeDeformation`
— `0.5F` hat, `0.25F` jacket/sleeves/pants. They agree.)

So our head is effectively correct (0.03 vs 0.03125) but **body, arms and legs are
nearly double Minecraft's** — an imported Minecraft skin renders with a chunkier
jacket and sleeves here than it has in Minecraft. Correcting that makes the gap
*thinner*, working against visibility: at ×4 blow-up a Minecraft-correct sleeve gap
is 0.06 world units, about a millimetre on screen.

### Finding 5: the hint line is a wall of text

One 130-character grey 13px string listing nine keys (`hud_ui.rs:1296`), which also
disagrees with its own doc comment about whether the picker is `C` or `O`.

The player guide (`cosmetics.md`) is otherwise clear and accurate but predates `O`
and `X`. **Documentation is not the fix** — the controls must not need it.

## Goals

1. Every part of the skin can be painted.
2. A kid can paint without visiting the inventory or learning nine keys.
3. It is impossible to paint something you cannot see.
4. Skins render and export identically to Minecraft.

## Non-goals

Per-part clothes toggles (Minecraft's Skin Customisation has six); touch support;
mannequin feel-knobs (anchor, yaw, ×4 scale) — those wait for a playtest.

---

## Design

### 1. `R` — limbs apart / together

A `limbs_apart: bool` on `SkinPaintSession`, live only while an Avatar project is
locked. When apart, each part takes a model-space offset:

| Part | Offset | Gap at ×4 |
|---|---|---|
| head | +0.25 Y | 1 block under the chin |
| body | anchor, unmoved | — |
| arm_L / arm_R | ∓0.25 X | 1 block clear of the torso |
| leg_L / leg_R | ∓0.125 X, −0.25 Y | 0.5 apart, 1 below the torso |

Eased over ~0.2 s so the limbs read as floating apart rather than teleporting, and
snapped back automatically on Pin.

**Verified:** this pose makes all 36 faces 100% reachable. A gap as small as 0.05 is
geometrically sufficient, so the sizes above are chosen for comfortable aiming with a
mouse, not for bare possibility.

**Lock-step constraint.** Render and hit-test must move together or the player aims at
ghosts. The geometry is currently duplicated — `skin_hit.rs:21` holds its own copy of
the boxes and `skin_hit.rs:31` its own copy of the inflate constant. The offsets
therefore live in **one new module, `skin_pose.rs`**, exposing:

```rust
pub fn part_offset(part: usize, apart: bool) -> [f32; 3];
```

consumed by both `entity_model::build_workshop_avatar_vertices` and
`skin_hit::ray_hit_avatar`. This extends the invariant the existing Task 3 round-trip
test already guards.

### 2. `Tab` — the paint panel

Toggled with `Tab`. Frees the cursor through the existing modal pattern
(`p0_ui_modal_open`, `game_loop.rs:5567`) that the `O` picker already established.

```
+---------- PAINT ------------+
|   (   colour wheel   )      |
|  [][][][][][][][]  16 dyes  |
|  [][][][][][][][]           |
|  recent [][][]    [ERASE]   |
|-----------------------------|
|  Clothes [ on  ][ off  ]  V |
|  Mirror  [ off ][ on   ]  M |
|  Brush   [1][2][3]          |
|  Limbs   [together][apart] R|
|  [undo Z]  [redo X]         |
|  [ Pin — wear it        P ] |
+-----------------------------+
```

Every button carries its letter shortcut, so the keys are learned by seeing them
rather than from the docs. The hint line collapses to:

```
Left-click paints  ·  Tab = colours & options  ·  P pin
```

Two consequences:

- **The eraser needs a real `erase: bool` on the session.** Erasing today means
  *holding nothing*, which cannot be expressed as a palette swatch.
- **Dyes become optional.** The held-dye path keeps working unchanged, but nothing
  forces an inventory trip. This is what makes the first thirty seconds work.

**Tab must stay contextual.** `Tab` is currently unbound, but a player list already
exists (`hud_ui::draw_player_inspect`, a collapsible "Players (N)" window,
`game_loop.rs:18188`), and hold-Tab is the conventional treatment for it on servers.
So `Tab` is **consumed only while a skin-paint session is open** — which requires
`world.is_workshop` *and* a locked Avatar project — and falls straight through
everywhere else. The Workshop can never be a server: it is filtered out of the lobby
world list (`menu.rs:590`, *"never a lobby world card"*) and hosting is only reachable
from a world card, so its roster is always empty. A future hold-Tab scoreboard needs
no rebinding exercise.

### 3. The clothes toggle replaces the layer concept

One `clothes_on: bool` drives three things at once:

- whether the overlay shell is **drawn**
- whether clicks **land** on it (`SkinLayer::Overlay` when on, `Base` when off)
- what the panel **says**

`V` stops being a layer switch and becomes the shortcut for this one toggle, matching
the panel. There is no longer a separate notion of "which layer am I painting" —
clothes visible *is* clothes selected, so painting something you cannot see stops
being expressible.

**The default is derived from the skin.** On session start, scan the overlay regions
of the working buffer:

```rust
/// True if any pixel in the 36 overlay rects has alpha > 0.
pub fn clothes_layer_has_content(buffer: &[u8]) -> bool;
```

using the exact rects from `skin_uv::overlay_faces()` (6 parts × 6 faces), never the
whole atlas.

- **Nothing painted → clothes default OFF.** A fresh skin opens on the body, with no
  invisible shell to be confused by.
- **Anything painted, even one pixel → clothes default ON.** An imported Minecraft
  skin with a hoodie opens showing the hoodie, ready to edit.

After that, the player's toggle wins for the session. The panel captions the state
(`off — nothing painted yet`) so the default explains itself.

**Empty-shell cue.** Turning clothes ON when the layer is empty is how you *start*
painting clothes, so that case still needs feedback: the shell renders as a faint
wireframe outline, showing where the strokes will land.

*Note:* alpha > 0 is the rule, faithful to "even just one square" — in-game dye
strokes are fully opaque. A lossily-imported PNG carrying near-invisible alpha dust
could in principle default a bare-looking skin to clothes-on; the toggle is one click.

### 4. Offset: exact when worn, exaggerated while editing

`OVERLAY_INFLATE` becomes per-part, matching Minecraft, plus an edit-only value:

```
head             0.03125    (1 px total)
body/arms/legs   0.015625   (0.5 px total)
WORKSHOP_EDIT    0.0625     (1 px per side — blown-up mannequin only)
```

The edit value gives 0.25 world units of daylight at ×4 — roughly 4× today's visible
gap — and shrinks back to the true value on Pin. Your avatar and every exported PNG
use the Minecraft-exact figures, so a skin looks identical here and in Minecraft.

Three call sites move together, sourced from one place:
`entity_model.rs:1387` (third-person + mannequin), `viewmodel.rs:122` (first-person
sleeve), `skin_hit.rs:31` (hit-test).

**Side effect to see before it ships:** this visibly thins the jacket and sleeves on
every skin that already exists, including Axo's current look. It is the correct value
and it fixes Minecraft-import fidelity, but it changes how existing content renders —
not just new content.

---

## Testing

The reachability sweep becomes a permanent Rust test — the guard that today's wall
cannot silently return:

1. **All 36 faces hittable in the exploded pose.** Ray-cast each face from outside;
   assert every `(part, face)` is reachable.
2. **Offsets match Minecraft.** Assert the per-part constants equal 0.03125 / 0.015625.
3. **Render/hit-test lock-step** in *both* poses — extends the existing round-trip test.
4. **Clothes off makes the overlay unhittable**, and clothes on makes the base
   unhittable — the "cannot paint blind" invariant, asserted directly.
5. **`clothes_layer_has_content`** — false on the default skin, true after a single
   overlay pixel, and unaffected by base-layer pixels.
6. **Tab is not consumed outside a paint session** — the non-collision guard.
7. Panel state round-trips (selected colour, eraser, brush, mirror).

`./check.sh` is the gate as always (clippy `-D warnings`, build, tests, trunk, bundle
size). `cargo test` needs `CARGO_INCREMENTAL=0` on this host.

## Docs

`cosmetics.md`'s control table and `workshop.md:5` are rewritten for the panel. The
current table is already stale (missing `O` and `X`). The aim is that the table
becomes a reference nobody needs to reach for.

## Risks

- **Existing skins render differently** (§4 side effect) — owner should see it first.
- **Geometry duplication** between `skin_hit.rs` and `entity_model.rs` is the standing
  hazard; `skin_pose.rs` reduces it to one source for offsets, but the box table
  itself stays duplicated. Consolidating that is a candidate follow-up, not in scope.
- **Native AppImage is not rebuilt** by this work — desktop users keep the old binary
  until the metered CI step runs (see `project_release_runbook`). Web deploys on push.

## Deferred

Per-part clothes toggles; touch/pinch painting; consolidating the `BOXES` table;
mannequin feel-knobs pending Axo's playtest.

---

### 2026-09-06 additions

Skindex parity, so a kid who learned on an online skin editor is at home:

- **`PaintTool` enum replaces `erase: bool`** — Brush / Fill / Eraser / Lighten /
  Darken / Noise. One tool at a time; colour precedence unchanged (Eraser → no
  colour; else picked colour beats held dye; no colour on Brush still erases).
- **Fill (`F`)** wires the long-dormant `skin_paint::fill` — floods the aimed face
  rect, mirrored when Mirror is on, and refuses with a message if no colour is picked.
- **Lighten / Darken / Noise (`B` cycles)** — ±10% toward white/black, or ±12%
  brightness jitter from a seeded LCG (no `rand`: wasm must build).
- **Shift+click** rules a Bresenham line from the last painted texel on the same
  part/face/layer. `line_texels` is canonical (computed from the smaller endpoint)
  so A→B and B→A paint identically.
- **Recent colours** (8, most-recent-first, dedup) and a **`#rrggbb` box** that
  commits only when it parses, via a session-held edit buffer.
- **Undo depth 24 → 64.**

**The once-per-stroke rule.** Lighten/Darken/Noise *modify* the texel already there,
so a per-stroke `touched: AHashSet<(u32,u32)>` on the session — cleared on the
mouse-down edge by `begin_stroke`, consulted by every shading op — applies each
texel at most once per stroke. Without it, holding the button on one spot compounds
to white/black, and a mirrored hit that folds onto the same texel (the centre column
of a front face) applies twice in a single click.
