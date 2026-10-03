# The Workshop

The Workshop is a private design room where you change how blocks **look** — both their colours (a *reskin*) and their shape (a *reshape*). Whatever you make here is worn by **every** block of that type, in every one of your worlds. It's purely visual: block behaviour, saves, and the rules of the game never change.

You also **paint your own avatar's skin** here: a mannequin of you stands in the room — blow it up with the Bellows, press **Tab** for the colours and options, and **R** to move the limbs apart so you can reach every side. See [Skins & the Wardrobe](cosmetics.md) for the full walkthrough.

New to this? The learn-journey lesson [Make It Your Own](https://learn.axenstax.com/docs/journey/learn-journey/make-it-your-own.md) walks you through your first reskin. This page is the reference — every control, in one place.

## Opening the Workshop

From the **lobby** (your world list), click the **🔧 The Workshop** button. The Workshop is a single room of its own — it's not one of your worlds, and it has its own save. The first time you open it, it's created as an empty void floor.

You're given a **Bellows** in your hotbar inside the Workshop. The Bellows is the blow-up tool, and it only works in here — it can't be crafted and does nothing in a normal world.

> To clear the room back to an empty floor, use **Reset The Workshop** from the menu. That wipes what you've *built* in the room but **keeps** your saved skins and redesigns (your Wardrobe survives a reset).

## The blow-up loop, at a glance

| Step | What you do |
|---|---|
| **1. Place** | Drop a normal block on the floor from your hotbar. |
| **2. Aim** | Hold the **Bellows** and look at the block — a cage outline appears. |
| **3. Blow up** | Hold right-click — the block swells to four blocks tall and locks. |
| **4. Edit** | With a **dye in hand**, paint cells or place coloured microblocks (Paint mode); press **V** for Sculpt mode to carve. |
| **5. Pin (P)** | Saves the design and wears it on every instance of that block. |

## The Bellows & the cage

Place a block, then **hold the Bellows and aim at it**. A **4×4×4 wireframe cage** shows exactly where it will grow:

- The cage is anchored at the block's **near-bottom corner** and grows **up and away from you** — never back into your face.
- It **flips live** as you walk around the block, so you choose the grow direction before you commit.
- **Green** = the whole space is clear and it'll fit. **Red** = something's in the way, and the blow-up **refuses** with a message (*"Not enough room — clear a 4×4 space to blow this up."*).

### Blowing it up

**Hold right-click** with the Bellows aimed at a green cage. It's like charging a block-break: the block **swells in three smooth steps** — ×2, ×3, ×4 — as the charge climbs.

- **Let go early** and it slides **all the way back to ×1**. Only ×1 and ×4 are stable — there's no resting at a half-size.
- Reach **×4** and it **locks**. Now you can release the button and walk all the way around it to edit.

Why ×4? Four blocks tall is the **tallest you can still reach every face from the floor** — so you never need to fly to paint the top.

> **Put it back:** **sneak + right-click** the Bellows on a blown-up block collapses it back to ×1 without saving.

## Editing in-world

Once a block is locked at ×4, you edit it with the crosshair. The working copy is a fine **16×16×16 grid** of quarter-block cells — one cell is one texel, so painting a pixel and placing a tiny block use the **same aim**.

**Paint vs Sculpt — press V.** The Workshop has two distinct modes, toggled with **V**:

- **Paint mode** (the default) — left-click with a dye paints; left-click with **no dye in hand does nothing**. Paint mode never carves, so you can't accidentally chip your block while colouring it.
- **Sculpt mode** — left-click **carves**, removing the microblock under your crosshair. You have to be in Sculpt mode to carve at all.

| Input | What it does |
|---|---|
| **V** | Toggle **Paint** / **Sculpt** mode — see above. |
| **Left-click** *(Paint mode, dye in hand)* | **Paint** the cell under your crosshair the dye's colour. |
| **Right-click** *(Paint mode, dye in hand)* | **Place** a new coloured microblock against the face you're aiming at. |
| **Left-click** *(Paint mode, no dye)* | Nothing — Paint mode never carves. |
| **Left-click** *(Sculpt mode)* | **Carve** — remove the microblock under the crosshair. |
| **G** | **Eyedropper** — grab the colour off the cell you're aiming at, ready to paint with. |
| **M** | **Cycle the mirror** (symmetry) mode — see below. |
| **P** | **Pin** the design (saves it and wears it everywhere). |
| **K** | Open the **Wardrobe** gallery — pick a block from the list to see its saved designs. |

You paint with **dyes** — hold the dye of the colour you want and left-click. There are 16 dye colours (see [Dyes, Fibre & Magnesium](dyes-fibre-and-magnesium.md)). The eyedropper (**G**) lets you copy a colour that's already on the block instead of digging for the right dye.

### Mirror (symmetry) — press M

Every block in the game has only **three** texture surfaces: a top, a bottom, and **one side shared by all four sides**. So a flat reskin never makes you paint a side four times — paint "the side" once and all four match. For 3D sculpts, **M** cycles a mirror mode so identical sides stay one action:

- **All sides** — your strokes mirror to the other three sides (the default; matches how blocks are textured).
- **Left–right** — sculpt one half, the other half mirrors.
- **Off** — when each side should look different.

A toast shows the current mode each time you press **M**.

## Pinning — press P

Aim at your blown-up block and press **P** to **pin** it. Pinning:

1. **Bakes** the design — a pure recolour becomes a texture, a shape change becomes a coloured micro-model.
2. **Wears it on every instance** of that block type, live, across your worlds.
3. **Saves it into that block's Wardrobe** (it never destroys an earlier design).
4. Deflates the block back to ×1 in one smooth motion.

## The Wardrobe — press K

Each block type has a **Wardrobe**: a set of saved designs you can keep, switch between, or throw away. Press **K** to open it — it opens showing whichever block you last picked in the panel, not automatically scoped to whatever you're currently editing (it works the same from chat with `/ws gallery`).

In the Wardrobe panel:

- The left column lists blocks you've designed. Pick one to see its saved designs.
- **Set active** — choose which design is *worn* on every instance of that block.
- **Use original** — go back to the block's stock art (wears no design).
- **Rename** — give a design your own label.
- **Delete** — remove a design you don't want.

Each block holds up to **16** designs. Pin a 17th and the oldest design you aren't currently wearing is dropped to make room.

> **Adopting others' designs.** A skin you adopt from someone else's Stash Column lands **in the Wardrobe, switched off** — open the Wardrobe (**K**) and **Set active** to wear it. It never silently replaces your active pick. See [Cloud Saves & the Stash Column](cloud-and-stash.md).

## Power-user chat commands

The blow-up gesture is the main way to work, but the older `/ws` chat commands still work as a precise fallback:

| Command | What it does |
|---|---|
| `/ws place <asset>` | Drop a project for a block or mob into the world. |
| `/ws edit <asset>` | Open that project on a 2D paint grid to edit it. |
| `/ws paint [id] <r> <g> <b>` | Set a solid reskin colour (0–255). |
| `/ws pin [id]` | Commit a project — every instance updates. |
| `/ws pump` / `/ws deflate` | Blow a project up to ×4 / shrink it back to ×1 from chat. |
| `/ws reshape <block>` | Start reshaping a block's 3D model from chat. |
| `/ws play [on|off]` | Toggle the mannequin preview on or off. |
| `/ws gallery` | Open the Wardrobe panel. |
| `/ws list` | List the Workshop's projects. |
| `/ws revert` | Revert all your reskins/reshapes back to stock art. |
| `/ws follow <npub>` | Follow a creator to see their published designs. |
| `/ws unfollow <npub>` | Stop following a creator. |
| `/ws following` | List the creators you follow. |
| `/ws browse` | Browse designs available to adopt. |
| `/ws adopt <id>` | Adopt a design into your own Wardrobe. |

## Sharing your designs

Your designs are **yours** — each carries your Nostr name (npub) and a record of what it was based on. You can publish a design for others to find and adopt, and a published design is signed so nobody can forge one under your name. Adopting someone else's design is safe: it's checked over before it ever loads. This sharing flow is **fully live today**, not a preview — use `/ws follow`, `/ws browse`, and `/ws adopt` from chat to find and pull in others' published designs (see the command table above). The sharing side rides the same Stash plumbing as cloud saves — see [Cloud Saves & the Stash Column](cloud-and-stash.md).

## Coming soon

> **Coming soon:** **Sound.** The pump, paint, and carve actions are silent for now — the rhythm is there, the audio isn't.

> **Coming soon:** **Reshaping mobs.** The blow-up gesture already works on **blocks** and on **your own avatar** (paint your skin — see [Skins & the Wardrobe](cosmetics.md)); using it to *reshape mobs* is still to come — for now reskin a mob's colours through the `/ws edit mob:<name>` paint grid.

## See also

- [Make It Your Own](https://learn.axenstax.com/docs/journey/learn-journey/make-it-your-own.md) — the hands-on first-reskin lesson.
- [Dyes, Fibre & Magnesium](dyes-fibre-and-magnesium.md) — the 16 paint colours.
- [Cloud Saves & the Stash Column](cloud-and-stash.md) — publishing and adopting designs.
- [Cosmetics & Skins](cosmetics.md) — changing your own avatar's look.
