# Food & Cooking

You're going to get hungry. Eating restores hunger; high hunger restores health. Cooking food at a campfire makes it heal more.

## Hunger

The drumstick row above your hotbar shows hunger from **0 (empty) to 20 (full)**. Hunger drains as you play — walking, jumping, attacking all cost a few hunger points over time.

- Hunger **≥ 18**: health regenerates slowly while you're not taking damage.
- Hunger **< 18**: no regen.
- Hunger **= 0**: hearts start dropping until you eat.

The hunger bar is on screen all the time so you can spot the slow drift downward.

## Eating

Put food in your hotbar. Select that slot. **Right-click** with nothing pointed at. The character eats one item — hunger ticks up by the food's value, and (if you needed it) some health comes back too.

Right-click only triggers eating if hunger is below max OR health is below max. Otherwise nothing happens — you don't waste food.

## Food values (raw → cooked)

| Food | Hunger | Notes |
|---|---|---|
| **Raw Beef** | 3 | From cows. |
| **Cooked Beef** | 5 | Beef + campfire. |
| **Raw Porkchop** | 3 | From pigs. |
| **Cooked Porkchop** | 5 | Pork + campfire. |
| **Raw Chicken** | 2 | From chickens. |
| **Cooked Chicken** | 4 | Chicken + campfire. |
| **Raw Mutton** | 2 | From sheep. |
| **Cooked Mutton** | 4 | Mutton + campfire. |
| **Bread** | 5 | 3 wheat in a row (crafting table). |
| **Carrot** (raw) | 3 | Harvested from a mature carrot crop. |
| **Baked Carrot** | 4 | Carrot + campfire. |
| **Potato** (raw) | 1 | Harvested from a mature potato crop. |
| **Baked Potato** | 5 | Potato + campfire. |
| **Corn** (raw) | 2 | Harvested from mature corn. |
| **Baked Corn** | 5 | Corn + campfire. Shows as "Corn on the Cob". |
| **Raw Fish** | 2 | Cast a **Fishing Rod** at water (right-click), wait for the bite, right-click again to reel in — or take it from the **Fish** mob (see [Combat & Mobs](combat-and-mobs.md)). |
| **Cooked Fish** | 6 | Fish + campfire. |
| **Berries** | 2 | From wild Berry Bushes. |
| **Loaded Baked Potato** | 10 | Baked Potato dressed up further. |

This table covers the common staples — there are more food items in the full catalogue than fit here.

## Campfires

The campfire is the alpha's cooking workstation. Place one, fuel it, light it, drop raw food on it. It cooks while you do other things.

### Placing + crafting

A campfire is crafted at a Workbench: 3 sticks across the top, 3 logs (any type) across the middle, 3 sticks across the bottom. Produces 1 **unlit** campfire — you fuel + ignite separately after placing.

Place it on a solid block. It starts **unlit**.

### Adding fuel

**Right-click** the campfire while holding a fuel item:

| Fuel | Burn time |
|---|---|
| **Oak Leaves** | 1 second |
| **Stick** | 2 seconds |
| **Oak Planks** | 12 seconds |
| **Green Log** | 30 seconds |
| **Oak Log** | 60 seconds |
| **Seasoned Log** | 60 seconds |
| **Kiln-Dried Log** | 120 seconds |
| **Coal** | 240 seconds (4 minutes!) |

Adding fuel **does not light the fire**. It just puts fuel in.

### Lighting the fire

Three ways:

1. **Flint and Steel** — right-click the unlit campfire. Lights instantly. Consumes 1 durability point on the F&S (it has 65 total).
2. **Magnesium Firestarter** — right-click the unlit campfire. Also lights instantly, and it's **not consumed** — a reusable striker. See [Dyes, Fibre & Magnesium](dyes-fibre-and-magnesium.md) for how to make one.
3. **Stick friction** — hold a stick, right-click the unlit campfire and **hold for 5 seconds**. The stick rubs against the wood. There's a 70% success rate; the stick is consumed either way. Try again if it fails.

Lighting requires **fuel in the campfire** — an empty unlit campfire won't catch.

Once lit, the campfire shows the **lit texture** (with flames + glow) and starts burning fuel.

### Cooking

With a lit campfire, **right-click** while holding a raw meat or a plantable vegetable:

- It enters one of the campfire's 4 cooking slots.
- After about 10 seconds it converts to its cooked variant.
- The cooked item stays in the slot until you collect it.

**Right-click without holding food** to collect cooked outputs. You get whatever's ready, one slot at a time.

A campfire can cook **4 items at once**. Cooked outputs don't expire — they wait for you.

### Burning leaves and the smoke pillar

When the campfire is lit AND a leaf is in the fuel pipeline, a **CAMPFIRE_SMOKE block column** appears above it. It rises a few blocks high and is visible from a long distance. Two effects:

1. **Other players can see your camp** from across a hill.
2. **Hostile mobs are drawn** to lit campfires (Spec 18 — they "investigate" the beacon). They stop at the **heat radius** — a circle proportional to fuel level. That keeps them at the edge, where the village Knight (or your sword) can deal with them.

### Cooling down (Spec 30 update)

When the fuel runs out the campfire enters a **smoulder window** — about **30 seconds** where it's visually unlit (no light, no heat) but **easy to relight**. Drop more fuel in during smoulder and it lights instantly — no friction or flint and steel needed. The fire "remembers" it was just hot.

After 30 seconds of smouldering with no fuel added, the campfire goes **fully cold** and needs the regular friction-stick or flint-and-steel ignition. Half-cooked items remain in their slots during smoulder + cold; you don't lose them, they just stop progressing.

### Hover label

Aim your crosshair at any campfire **within striking distance** (~4 blocks) and a floating label appears showing:

- **Mode**: Ready to light / Lit / Smouldering / Cold
- **Fuel remaining**, shown as seconds (never raw ticks)
- **Heat radius** in blocks — only shown while **Lit**
- **Smoke** on/off

Use this to know whether to walk away or top up the fuel.

If you break a lit campfire, it drops the unlit version as an item, and anything in its cook slots spills out too — a slot still cooking drops its raw item, a slot that finished cooking drops the cooked item, same as breaking a Furnace mid-smelt.

## Furnaces

The Furnace smelts **ore** — and only ore. As of Spec 29 (2026-05-21), the furnace is no longer used for cooking food; the **campfire** is the canonical cook station. Use the furnace when you have a backlog of raw iron, copper, or tin.

### Placing + crafting

Crafted at a Workbench: **8 cobblestone in a hollow ring** (Minecraft standard). Yields 1 Furnace.

Place it on a solid block. It starts idle (dark front).

### Opening + loading

**Right-click** the Furnace to open its UI. Then click the slots:

- Click **Input** while holding ore (raw iron / copper / tin) → 1 unit moves into Input. **Shift-click** moves the whole stack.
- Click **Fuel** while holding coal / planks / logs / sticks → 1 unit moves into Fuel. Shift-click moves the stack.
- Click **Output** to take the smelted ingot back into your inventory.

**Press 'E' (or Esc)** to close the Furnace UI — same key that closes the inventory.

The fuel ladder matches the campfire: a stick is 2 s of burn; coal is 240 s. The Furnace smelts one item per **10 s** — so a piece of coal smelts 24 items if you keep the input slot stocked.

When fuel + input + output room are all present, the Furnace **lights** (front glows orange) and the smelt progresses.

### Smelt recipes (ore-only)

| Input | Output | Time |
|---|---|---|
| Raw Iron | Iron Ingot | 10 s |
| Copper | Copper Ingot | 10 s |
| Tin | Tin Ingot | 10 s |

**Raw meat does not smelt in the furnace.** Cook it at a campfire instead.

### Legacy save behaviour

If you load a save from before Spec 29 with raw meat sitting in a furnace input slot, the meat is ejected as a loose item near the furnace on first tick — you don't lose your beef, you just have to pick it up off the floor and walk it over to a campfire.

### Proof of Play trickle

On Bitcoin-enabled servers, each completed smelt fires a **1-sat Proof-of-Play trickle** to the closest player within 16 blocks (provided their Charter sats flag is on). It's a passive background credit — no toast, just a number that builds up in the Treasury panel. Off-Bitcoin servers and Charter-disabled players see no sats; smelting still works exactly the same.

### Breaking + spillage

Mining a Furnace returns the **idle** form as an item (same lit/unlit normalisation as the campfire) plus the **input + fuel + output stack contents**. So you can move a partly-loaded Furnace without losing materials.

## Tips

- A first-night kit is: **1 sword + 5 cooked porkchops + 1 lit campfire**. The campfire keeps mobs at a polite distance.
- Cooking is a great background task. Set up a campfire near your mining area, drop raw meat on it as you collect it, grab cooked food on the way out.
- **Bread** is cheap. 3 wheat → 1 bread. If you have a farm, you have a sandwich shop.
- Don't eat raw meat when you have cooked. Raw meat heals less than cooked — cook it first when you can.
