# Building Blocks — Slabs, Stairs, Doors & More

The **shaped blocks**. Not every block is a full cube — this family lets you
build at **half-block detail**: lay a slab for a step, run stairs up a slope,
hang a door you can open, fence a garden with a gate, glaze a window with panes,
top a wall, and label or decorate a room with signs and item frames.

> **New to it?** Do the hands-on lesson first:
> **[Build with Shapes](https://learn.axenstax.com/docs/journey/learn-journey/build-with-shapes.md)**.
> This page is for *looking things up*.

## How shaped blocks work

A full cube only ever sits one way. A shaped block remembers **which way it
faces** and, for some, **whether it's open or shut** — so *how you place it* and
*where you click* matters. Every one of these is **craftable**, places with a
sensible orientation, and saves exactly as you left it (open doors stay open).

You can grab any of them for testing from the chat (**T** or **/**) with
`/give <name>` — the names are in the table at the bottom.

## Slabs & stairs — half-height building

| Block | Recipe | Yield |
|---|---|---|
| **Stone Slab** | **3 Stone** in a row | 6 |
| **Stone Stairs** | 6 Stone in a **staircase** shape (left- or right-hand) | 4 |

- **Slabs** fill **half a block**. *Where you click decides which half:*
  - Click the **top** of a block → the slab rests on the **floor** (bottom slab).
  - Click the **underside** → the slab hangs from the **ceiling** (top slab).
  - Click the **middle of a side** → a **vertical slab** stands against that side
    — and yes, all **four** vertical orientations work, so you can make
    half-thick walls.
  - Click **high or low on a side** → a bottom/top horizontal slab (the same as
    clicking the top/underside).
- **Stairs** face **the way you're looking** when you place them, so the step you
  walk up is in front of you. Place one against the **underside** of a block to
  get an **upside-down** stair (handy for arches and trims). You can walk
  straight up a run of stairs — no jumping.

## Doors, trapdoors & gates — things that open

| Block | Recipe | Yield | Open with |
|---|---|---|---|
| **Oak Door** | a **2-wide × 3-tall** column of Oak Planks | 3 | right-click |
| **Oak Trapdoor** | **6 Oak Planks** (a full 2×3) | 2 | right-click |
| **Oak Fence Gate** | **Sticks · Plank · Sticks** over **Sticks · Plank · Sticks** (2×3) | 1 | right-click |

- A **door** is **two blocks tall** and fills the whole doorway when shut. **Right-click**
  swings it open against the side wall so you can walk through; right-click again
  to shut it. Build **two doors side by side** and they make a **double door**
  that opens as a pair.
- A **trapdoor** is a thin flap. Shut, it's a lid on the floor (or the ceiling,
  depending how you place it); open, it swings flat against the wall. Great for
  hatches, hidden holes, and chicken coops.
- A **fence gate** is the doorway in a fence. Shut, it blocks the way like a
  fence; open, you walk straight through. (Plain fence posts are in
  **[Blocks & Mining](blocks-and-mining.md)** — the gate is the bit that moves.)

## Walls, panes & bars — windows and barriers

| Block | Recipe | Yield |
|---|---|---|
| **Cobblestone Wall** | a **3-wide × 2-tall** block of Cobblestone | 6 |
| **Glass Pane** | **6 Glass** (a full 2×3) | 16 |
| **Iron Bars** | **6 Iron Ingots** (a full 2×3) | 16 |

These three are **connecting** blocks — they're clever about their neighbours. A
lone one is just a thin central post; put another next to it (or against a solid
block) and an **arm grows out to join it**, automatically. Break the neighbour
and the arm goes away. So a row of glass panes becomes a **flat window sheet**, a
line of walls becomes a **proper wall**, and iron bars make a **cage or window
grille** — with no fiddling, they just connect.

- **Glass Panes** are thin glass — you see through them, and they're far cheaper
  per block than full glass for windows.
- **Iron Bars** are the metal version — a see-through barrier you can't walk or
  shoot through.
- **Cobblestone Walls** are knee-to-chest height with a fat post — fences for
  castles and gardens.

## Signs & item frames — labels and display

| Block | Recipe | Yield | Use |
|---|---|---|---|
| **Oak Sign** | **6 Oak Planks (2×3)** over a **centred Stick** | 3 | right-click to write; reads when you look at it |
| **Item Frame** | an **8-Stick ring** around **1 Leather** | 1 | right-click to mount an item |

- **Place a sign**, then **right-click** it to open a little text editor and type
  your message. Look at the sign in the world and the text shows on your screen —
  label chests, mark paths, leave notes.
- **Place an item frame** flat on a wall (it mounts on the face you click), then
  **right-click with an item** to display it — the item shows as a small floating
  cube on the frame. Decorate, or show off what you found.

## Levers, buttons & pressure plates

You'll find these in the **[Electricity — Power & Logic](electricity.md)** guide —
that's where they do their job (switching circuits on and off). The
building-blocks update gave them their **proper flush shapes**: a button is a
small nub on the wall, a lever has a real handle that flips, and a pressure plate
is a thin floor tile. How they *look* is here; what they *power* is on the
Electricity page.

## Everything at a glance — `/give` names

| Block | `/give` name(s) |
|---|---|
| Stone Slab | `slab`, `stone_slab` |
| Stone Stairs | `stairs`, `stone_stairs` |
| Oak Door | `door`, `oak_door` |
| Oak Trapdoor | `trapdoor`, `oak_trapdoor` |
| Oak Fence Gate | `fence_gate`, `oak_fence_gate` (note: `gate` alone gives the Electricity **Logic Gate**, not this) |
| Cobblestone Wall | `wall`, `cobble_wall`, `cobblestone_wall` |
| Glass Pane | `pane`, `glass_pane` |
| Iron Bars | `bars`, `iron_bars` |
| Oak Sign | `sign`, `oak_sign` |
| Item Frame | `frame`, `item_frame` |

> **One family, oak & stone for now.** Doors, gates, trapdoors and signs come in
> **oak**; slabs and stairs in **stone** — more woods and stones are coming. The
> way they place, open and connect is the same whatever they're made of.
