# Build Schematics

Build something you love, then keep it forever. The Build Schematics
system lets you **capture** a building into a plan you carry in your bag,
**develop** that plan in the sun, and **build** an exact copy of it
anywhere you like — as many times as you want.

Every plan remembers exactly what you made: the same blocks, in the same
shape, at the same size. It also remembers **who** made it, what
**licence** it carries, and whether it was designed in Survival or
Creative.

Want the hands-on version? See
[Capture your first schematic](https://learn.axenstax.com/docs/journey/learn-journey/capture-a-schematic.md).
This page is the reference — it explains how every part works.

## The lifecycle at a glance

| Step | What you do |
|---|---|
| **1. Capture** | Lay **Blueprint Paper** under your build, then right-click your build with the **Drafting Stamp**. |
| **2. Develop** | Lay the captured plan out under open sky in daytime until it turns blue. |
| **3. Build** | Hold the developed plan, place a ghost preview, then choose: have it built for you, or be **guided** through building it yourself. |
| **4. Plaque** | A plaque appears recording who made it — read it, or tip the architect. |

## 1. Capture

A capture is **1:1** — it copies the exact blocks at the exact size.
Nothing is shrunk or scaled.

**Blueprint Paper** is the parchment you lay around your build. Craft it
first (see [Crafting](crafting.md) for the recipe).

To capture:

1. Lay **Blueprint Paper** flat on the ground around or under your build.
   It's placed like normal blocks. The ground under each paper tile must
   be solid, with air directly above it.
2. Hold the **Drafting Stamp** (a reusable tool — craft it, or grab one in
   Creative) and **right-click any block of your build**. The build sits over
   the paper, so the Stamp scans straight down to find the tile underneath.
   (The old empty-hand gesture was retired on 2026-06-04 — the Stamp is the
   trigger now, and it is **not** used up.)
3. The capture floods across all the connected paper tiles at that level
   to work out the footprint, then captures everything built above them.
4. A dialog opens so you can **name** the plan, **pick a licence**, and
   save it.

✅ On save, the captured plan goes into your inventory as an **item** —
not a block. The **Blueprint Paper is used up** (there's no refund), so
only confirm when you're happy.

Capture works **anywhere**, even deep underground — you don't need any
sunlight to capture, only to develop later.

> **Changed your mind before capturing?** Mine a paper tile with any tool
> to peel it back up and reclaim it — no Eraser needed.

**Prefer wall art?** The **Building** capture above isn't the only flow —
a separate **Art** capture mode captures a vertical wall slice instead,
for hanging 2D scenes as prints. It's a distinct capture mode from the
Building capture described here.

**Already have a Minecraft build?** You don't have to capture it in-game
at all — you can import a Minecraft schematic (`.schem`, Sponge v2/v3)
file straight into a Plan.

**Limits and refusals**

| Limit | Value |
|---|---|
| Footprint | up to **32 × 32** |
| Height | up to **32** |

The capture will politely refuse if:

- ⚠️ the paper tiles aren't all connected,
- ⚠️ the ground isn't flat (each tile needs solid-below and air-above),
- ⚠️ there's nothing built above the paper, or
- ⚠️ the build is too large.

## 2. Develop

> **Creative mode:** captures are **developed instantly** — skip this step and
> go straight to *Build*. Developing in the sun is a **Survival**-only step.

A fresh capture is **Latent** — undeveloped. Think of it like
old-fashioned photo paper: it needs sunlight to bring the picture out.

To develop it:

1. Hold the Latent plan and right-click the ground. This paints a
   paper-thin **Blueprint** attachment onto the top face of that block —
   it's not a placed block of its own, just a flat floor overlay (so a
   captured build doesn't end up a block taller than it should be).
2. Leave it where it gets **direct daytime sunlight under fully open
   sky**. A roof, a tree canopy, or night-time **pauses** developing.
3. It needs about **90 seconds** of cumulative sunlight. Progress is
   kept across pauses — shade and night just put it on hold.
4. The plan's icon shifts through **four colour stages** to a finished
   blue when it's done.

**Mine the floor tile** underneath at any time to peel the plan back
off — the first strike lifts it back into your inventory at its current
progress.

⚠️ Carrying a Latent plan around in your bag does **not** develop it —
only a Blueprint attachment laid out under the sky develops.

## 3. Build (and rebuild)

Hold a **developed** plan and right-click to open the Inspect dialog,
then choose **"Place in world"** to enter a ghost preview.

| Control | What it does |
|---|---|
| Move / aim | Position the footprint where you want it. |
| **Q** / **E** | Rotate the plan 90°. |
| Wireframe colour | Green ✅ ok, yellow ⚠️, red ⚠️ blocked. |
| **Left-click** | Confirm the spot (on a non-red spot). |
| **Right-click** | Cancel the preview. |

The site must be flat, the space must be empty, and no player can be
standing inside it.

### What happens when you confirm

Confirming doesn't always build it for you — it depends on your mode and
what's in your bag.

**Short of materials in Survival?** The plan is laid out as a **build
guide** instead: a ghost of the finished thing, with a panel telling you
exactly what to **go and gather**. Nothing is built for you and nothing
is taken from your bag. Go and mine what you need, come back, and place
the blocks yourself — each one turns solid as you get it right.

**Got everything (or playing in Creative)?** A small panel asks how you
want it built:

| Choice | What happens |
|---|---|
| **Build it automatically** | The animated builder does the whole thing (see below). |
| **Guide me — block by block** | One block at a time. The next block glows; place it and the guide moves on. |
| **Guide me — layer by layer** | One whole floor-to-ceiling course at a time — like following instructions in a model kit. |
| **Cancel** | Nothing is placed and you keep the plan. **Esc** does the same. |

Press **Esc** at any time to back out.

### Building it yourself — the guide

The guide is the fun way to build someone else's plan: you do the
building, it does the remembering.

- The blocks you should place **right now glow bright white**; what's
  coming later shows faintly, so you can see the shape without being
  swamped by it.
- Put the **wrong** block somewhere and that cell turns **red** — the
  step stays put until you fix it.
- The panel reads **"Step 3 of 12"** and lists what this step still
  wants. In Survival it also shows a **"Go and gather"** list of the bits
  you haven't got yet.
- Get everything right and the guide clears itself with a well done.

Want to switch between block-by-block and layer-by-layer part way
through? Type `/buildguide mode block`, `/buildguide mode layer`, or
`/buildguide mode whole` (whole shows the entire plan at once, the
old-style reference ghost). You can also just lay the plan again and
pick differently — the plan stays in your bag.

### Building it automatically

Choose **Build it automatically** and the build plays out as an
**animated construction**, placing about **2 blocks per tick** from the
floor upward. It's tracked internally — no marker block appears in the
world.

**Survival vs Creative**

| Mode | Materials |
|---|---|
| **Survival** | You must have **all** the required blocks. They're checked and locked from your inventory up front, then used as the build places them. Cancel mid-build and the unplaced materials are refunded. ✅ |
| **Creative** | The material cost is skipped entirely. |

> The material check only applies to the **automatic** build. When you're
> being **guided**, blocks leave your bag as you place them, exactly like
> building anything else — so you can start with a half-empty bag and
> top it up as you go.

Rebuilds are **1:1** — the same size as captured. Only rotation is
applied.

## 4. Plaque & attribution

When the build finishes, an **Architect Plaque** is placed automatically
at the centre of the build. It's engine-placed — you don't craft it.

The plaque records the full **derivation and credit chain**:

- each architect's Nostr name (npub),
- the plan's name,
- its licence, and
- whether it was authored in Creative or Survival.

Right-click the plaque to read the chain. Each architect has a
**"Tip 1 sat"** button next to their name.

> **Coming soon:** tipping shows up on Bitcoin-enabled servers, for
> architects who have a signed name.

**Licences**

The default is **CC-BY-SA**.

| Licence | What it means |
|---|---|
| **All Rights Reserved** | The architect keeps all rights. |
| **CC0** | Public domain — anyone can use it for anything. |
| **CC-BY-SA** | Credit the architect; any derived plan stays CC-BY-SA. *Default.* |
| **CC-BY-ND** | Credit the architect; no modified versions. |

If you save a plan as a **derivative** of another, the parent's credit
chain is prepended to yours. Parents are auto-detected automatically — a
new capture that's a **50% or greater partial match** to an existing
plan (not just an exact copy) is linked as a derivative, so the original
architect always keeps their place in the chain.

## Other building tools

Want direct region editing instead of the capture-and-place workflow?
`/we` gives you WorldEdit-style chat commands for selecting and editing
regions directly — handy for big terrain edits outside the Plan flow
above.

## Resume

Interrupted builds resume on their own. If you quit mid-build — or
mid-develop — the progress and any locked materials are saved with your
world. When you reload, the build carries on from where it left off.

There's no manual resume button. ✅

## Hang a Cyanotype Print

Hold a **developed** plan and right-click a **wall** (a vertical face) to
hang a **Cyanotype Print** — framed wall décor. This uses up the plan.

> **Note (v1):** the hung print shows a generic blueprint-blue picture,
> not yet a picture of your specific build.

## See also

- [Capture your first schematic](https://learn.axenstax.com/docs/journey/learn-journey/capture-a-schematic.md) — the step-by-step tutorial.
- [Crafting](crafting.md) — the **Blueprint Paper** recipe.
