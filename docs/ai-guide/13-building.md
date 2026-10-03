<!-- SOURCE: block_shape.rs, sign.rs, item_frame.rs, plan.rs, blueprint_attach.rs, build_guide.rs, schematic.rs, workshop.rs, workshop_painter.rs, rig_studio_ui.rs, block.rs, input.rs, crafting.rs | Verified against code 2026-06-22 -->

# 13 — Building

Purpose: the verified facts on how a player shapes, decorates, copies and re-skins their world — shaped blocks (stairs/slabs/walls/etc.), signs and item frames, blueprints (capture + rebuild), the Workshop editor + painter, and animated rigs. Everything below is confirmed in engine source. UK English. Anything not listed here is **not in the game** — say so kindly.

---

## 1. Shaped blocks (not just full cubes)

Most blocks are full cubes. Some blocks have a **special shape** and a **facing** chosen when you place them. The complete shaped set the engine knows (from `block_shape.rs` → `BlockShape`):

| Shape | Block(s) in the game | What it is | Collision |
|---|---|---|---|
| **Slab** | Stone Slab | A half-block. Can sit as bottom, top, OR one of four vertical halves. | Half-box |
| **Stairs** | Stone Stairs | A step: a half-slab + a quarter on top, faced your way. Can be upside-down. | Two boxes |
| **Fence gate** | Oak Fence Gate | A panel across a gap. Right-click to open (you walk through) / close (blocked). | Blocks when shut, clear when open |
| **Trapdoor** | Oak Trapdoor | A thin flap. Right-click to open/close; mounts at floor or ceiling. | Thin lid / thin flap |
| **Pane** | Glass Pane, Iron Bars | Thin upright sheets that **auto-connect** into a flat run. | Thin |
| **Door** | Oak Door | A thin panel **two blocks tall**. Right-click to swing open/shut. | Blocks doorway when shut |
| **Wall** | Cobblestone Wall | A post that **grows arms** toward touching walls/solid blocks. | Post + arms |
| **Button** | (Button) | A small nub on the clicked face. *Shape only — power behaviour is the Electricity feature.* | Pass-through |
| **Lever** | (Lever) | A handle that flips when right-clicked (**visual only** right now). | Pass-through |
| **Pressure plate** | (Pressure Plate) | A flat plate on the floor. *Shape only for now.* | Pass-through |
| **Sign** | Oak Sign | A text board on a post (see §2). | Pass-through |
| **Item frame** | Item Frame | A thin plate that displays an item (see §2). | Pass-through |

Everything else is a normal full cube — placed flat, no orientation to think about.

### How shaped placement works
- **Ghost preview:** when you hold a blueprint *plan* ready to place, a wireframe **ghost** follows your cursor so you can see where it lands. It is colour-coded: **green** = good to build, **yellow** = good spot but you're short on blocks (survival), **red** = can't place here. (This ghost is for **blueprints/plans** — ordinary shaped blocks just place where you aim.)
- **Rotate the ghost:** with a plan ghost active, **Q** rotates it 90° anticlockwise and **E** rotates it 90° clockwise. *(Keyboard only — see Deferred.)*
- **Slabs** orient from where you click:
  - Click the **top** of a block → a **bottom** slab (rests on it).
  - Click the **bottom** → a **top** slab.
  - Click a **side** in the middle → a **vertical** slab against that face.
  - Click a side **high or low** → a top/bottom horizontal slab.
- **Stairs** face the way you're looking, so the tall back is behind the step you walk up. Click a block's **underside** to place an **upside-down** stair.
- **Walls, glass panes and iron bars connect by themselves.** Place two next to each other (or next to a solid block) and they join into a continuous run — you don't set this; it updates live as neighbours change.
- **Openable blocks** (fence gate, trapdoor, door) **toggle on right-click** — open lets you pass, shut blocks the way. The facing is remembered through the toggle.

---

## 2. Decoration: signs and item frames

### Signs (Oak Sign)
- A standing **text board on a post**. It does not block you — you walk past it.
- **Right-click to open the sign editor and type your text.** Looking at a placed sign shows its text.
- Up to **120 characters**, and you can use **multiple lines** (press enter for a new line). Empty signs are allowed.
- The text is saved with the world, so your sign keeps its message after you reload.

### Item frames (Item Frame)
- A **thin plate mounted flush on a block face** that **displays one item**.
- **Right-click an empty frame while holding an item** to mount it (always a single item, even from a big stack).
- **Right-click a filled frame to rotate** the displayed item — there are **8 rotation steps** (45° each).
- **Break the frame to drop the item back.** Block items show as a small cube on the plate; other items show when you look at the frame.
- Frame contents persist with the world.

> Decoration you CAN colour: there is a set of **16 coloured "wallpaper" decals** (white, black, red, blue, yellow, orange, green, purple, pink, lime, light blue, grey, light grey, brown, cyan, magenta) you can paint onto a block face. These come from the 16 dyes (one dye → one colour).

---

## 3. Blueprints — copy a build, then rebuild it

You can **capture a structure you've built into a paper blueprint (a "Plan")**, then place that Plan elsewhere to rebuild it. This is the in-game schematic system (`plan.rs`).

### The Drafting Stamp + Blueprint Paper flow
1. **Blueprint Paper** is laid **flat on the floor** around the base of your build. It lays as a thin sheet on the **top face** of a floor block — it places **nothing in the air above** and never blocks you (it's a decal, not a cube).
2. **Craft a Drafting Stamp** (an Iron Ingot on top of Blueprint Paper). It's a wooden-tier tool with **32 uses**.
3. **Stamp any block of your build** with the Drafting Stamp. It scans straight down to find the paper sheet beneath, then **floods through your connected, solid build** to capture it.
4. A **Capture dialog** appears (name, licence — default Creative-Commons share-alike). **Confirm** turns the paper into an **Item: Plan** in your inventory.

Notes the assistant should get right:
- The capture grabs **connected, solid blocks** sitting above the papered footprint, up to the roof/apex. Loose, disconnected bits aren't pulled in.
- **Size limit:** the footprint is at most **32 × 32**, and the captured height at most **32**. Bigger than that and capture refuses ("too large"). It's a **house-scale** tool, not for mega-builds.
- A capture can refuse if: the build is too large, the paper tiles aren't all connected, or there's nothing built on the paper.
- **Lay a finished Plan flat on a floor** and it shows as a blueprint decal (a "developed" cyanotype blue) you can re-stamp.

### Rebuilding from a Plan
- **Place a Plan** and a **ghost wireframe** previews it (green/yellow/red as in §1). **Q/E rotate** it in 90° steps before you commit.
- Confirm and the structure **builds itself a couple of blocks at a time** (an animated build). In survival it spends the matching blocks from your inventory; in creative it builds from nothing.
- **Build guide:** the engine can show a **material list** (which blocks, how many) and a **per-block check** of what you've placed correctly vs what's still missing or wrong. This helps you copy a build by hand.

### Importing a Minecraft `.schem`
The engine can read a **Sponge `.schem`** file (the common Minecraft schematic format) and turn it into a Plan. Block types are mapped to the closest AxeNStax block; anything it doesn't recognise becomes stone so the **shape** is kept. (`.litematic` is not supported.) This is an advanced/desktop path — not something to send a kid to do casually.

---

## 4. The Workshop — re-skin and reshape your blocks

The **Workshop** is its own saved world you enter from the Lobby (`workshop.rs`). It's where you **change how an existing block (or a creature) looks for your whole game** — a re-skin you pin once and it applies to *every* one of that block.

The core gesture:
1. Stand a block (or creature "mannequin") on the Workshop floor.
2. **Blow it up to a big working size** with the **Bellows** tool (hold it on the block — it inflates up to ×4, like blowing up a balloon, and locks there).
3. **Edit it** (paint its colours, or sculpt its shape).
4. **Put a pin in it** — it deflates and your new look is saved as the global appearance.

Un-pinned projects stay parked, inflated, and **persist with the Workshop world** — you can leave one half-finished and come back to it. Pinning is finishing one when *you* decide.

### Painting (the face painter)
- Each face is shown as a big **16 × 16 paint grid** — six grids, one per cube direction, seeded from the block's current texture so you tweak what's there.
- A **16-colour palette**: near-black, grey, white, red, orange, yellow, lime, green, cyan, blue, deep blue, purple, magenta, brown, tan, pink.
- Click (or drag) a grid cell to paint it. **Pin — apply to every one** commits it; **Cancel** discards.

### In-world Workshop tools (the editor on the blown-up copy)
While editing a locked, blown-up copy, these tools exist. **All of them are keyboard keys** — note the touch/gamepad gap in Deferred:

| Tool | Key | What it does |
|---|---|---|
| **Eyedropper** | **G** | Grab the colour from the cell under your crosshair. |
| **Symmetry toggle** | **M** | Cycle mirror mode: **off → left-right → all-sides** (paint one stroke, it mirrors). |
| **Mode toggle** | **V** | Switch between **Paint** (recolour) and **Sculpt** (carve away / add coloured micro-blocks). |
| **Pin** | **P** | Pin the locked working copy — commit the redesign and deflate. |
| **Wardrobe / gallery** | **K** | Open the Wardrobe panel — your saved designs library (also `/ws gallery`). |

Paint is the safe default: you can't accidentally carve a block's shape — you have to switch to Sculpt deliberately (V). Once you sculpt a block into a new shape there's no path back to a flat texture.

---

## 5. Rigs — build an animated creature (Rig Studio)

The **Rig Studio** (`rig_studio_ui.rs`, **Y key** to open) lets you **build your own animated creature out of blocks**:
1. Pick a **skeleton** (e.g. two-legged / four-legged).
2. Pick a **motion** — **Walk**, **Idle**, or **Bounce** (a gentle squash-and-stretch breathe).
3. **Assign a block to each body part** (hold a block, click "Set" on that part).
4. **Spawn the rig** — it stands in the world as an animated display that moves with the skeleton's built-in gait. A part whose block has a **micro-model** (a baked mini sculpture) is drawn as that sculpture, scaled to fit the limb, rather than a plain box.

Placed rigs are real world objects: they **persist with the world save**, so an animated creature you build is still there after you reload.

---

## Deferred / not yet — and the touch/gamepad gap

- **Workshop tools and a few build helpers are keyboard-only.** On **touch and gamepad**, these specific actions are **not currently bound**, so a player on a phone or controller can't reach them — do **not** instruct them to use these there:
  - **Rotate ghost** (Q/E), **toggle Inventory Explorer** (B),
  - and the Workshop tools: **eyedropper (G)**, **symmetry (M)**, **mode toggle (V)**, **pin (P)**, **Wardrobe/gallery (K)**.
  Keyboard players have all of these.
- **Buttons, levers and pressure plates are shape-only.** They render as proper button/lever/plate shapes and a lever's handle flips when right-clicked, but their **power / signal behaviour is part of the Electricity feature** — don't promise that a button or pressure plate *does* anything yet beyond looking the part. (The lever flip is visual only.)
- **`.litematic` import** is not supported — only Sponge `.schem`.
- **Blueprint capture is house-scale only:** max 32×32 footprint, max 32 tall. Don't tell a player they can blueprint a huge build.
- **Mob blow-up sculpting in the Workshop is limited** in this version (block targets are the fully-supported in-world sculpt path); a mob can be re-painted via the face painter, but treat mob *reshaping* as not a reliable activity to send a player to.

When a player asks for something here that isn't in the lists above, say kindly it isn't in the game yet, and offer the closest thing that **is** — there's a lot they can build with what's here.
