# Inventory & Tools

Your inventory has 36 slots: 9 in the **hotbar** (always visible at the bottom of your screen) and 27 in the **main inventory** (visible only when you press **E**).

## Hotbar

The 9 slots at the bottom of your screen. Only the selected slot is "in your hand" — the highlighted one with a brighter border. Use:

- **Number keys 1–9** to pick a slot directly.
- **Scroll wheel** to cycle.

The block name + count appears just above the hotbar so you always know what you're about to use.

## Main inventory

Press **E** to open it. You'll see a 2×2 crafting grid + your hotbar + a 27-slot bag. **Hover** over any slot to see the item name (e.g. "Iron Pickaxe", "Cobblestone (32)").

To **pick up** a stack: left-click it. The cursor now carries it — you'll see the icon float with your mouse and a label tells you what you've got.

To **place** the carried stack: left-click into any slot. If the destination is empty, the whole stack lands there. If it's the same item type, they merge up to the max stack size. If it's a different item, the two stacks swap.

To **close** the inventory: press **E** again or **Esc**. Any item you were carrying drops back into your bag automatically.

## Inventory Explorer (B key)

Press **B** at any time to open the **Inventory Explorer** — a big browse-everything pane that shows every block, tool, and material in the game. Useful when:

- You're trying to remember what something is called.
- You want to see what's coming up that you haven't found yet.
- You're in **creative** and want to grab a specific item without typing `/give`.

**How it works:**

- The search box at the top is focused when the pane opens — just start typing. The list narrows live as you type. "iron" finds Iron Pickaxe, Iron Ore, Iron Ingot, Iron Block.
- The category buttons (**All / Blocks / Tools / Materials**) narrow further.
- In **creative**, click any entry → one of that item lands in your inventory.
- In **survival**, hover over an entry → it shows **×N** next to the name where N is how many you currently hold. Great for "wait, do I have any bone meal?"

Press **Esc** to close.

## Materials roster (what's in the game)

Materials are things you carry that aren't blocks and aren't tools. Sourced from mining, mob drops, crafting, or farming. The full list, grouped by source:

- **Mob drops**: Leather (cow / Marauder / Berserker / Bear), Feather (chicken), Wool (sheep / Brigand), Bone (Bear / Hyena / wolf / livestock), Iron Ingot (the bandit family), Raw Beef / Pork / Chicken / Mutton, Brigand Chieftain Trophy (Berserker).
- **Mining**: Coal, Raw Iron, Diamond, Satori (the top gem, from pure-deepslate veins), Sulphur, Saltpetre, Magnesium, Salt, Flint (from gravel). Copper, Tin, and Amethyst exist as materials but their ores don't generate underground yet — see [Blocks & Mining](blocks-and-mining.md).
- **Smelted ingots** (in the furnace): Iron Ingot, Copper Ingot, Tin Ingot, Bronze Ingot (Copper + Tin alloy — decorative for now).
- **Crafted intermediates**: Stick, Bone Meal, Arrow (stick + feather), Sugar.
- **Farming Tier 1**: Wheat Seeds → Wheat → Bread; Carrot; Potato; Corn (separate seeds + ear). Baked Potato / Baked Carrot / Corn on the Cob via campfire.
- **Wood economy**: Green Log (fresh from a tree mine) → Seasoned Log (after drying rack) → Kiln-Dried Log (future Kiln output).
- **Papyrus**: Papyrus Reed → Papyrus Sheet (3 reeds = 3 sheets). First paper material in the engine.
- **Cooking & processing** (data in the engine; live workstations coming): Bucket, Milk Bucket, Egg, Flour, Dough, Cream, Butter, Cheese, Sweet Bread, Cake, Pumpkin Pie, Berry Pie, Cookie, Pancakes, Loaded Baked Potato, Stew, Beetroot Soup, Bowl, Sugar Beet, Beetroot, Pumpkin, Berries.
- **Saplings** (data in the engine; live planting + tree-gen coming): Oak / Birch / Spruce / Jungle / Acacia / Dark Oak Sapling.
- **Bee Hive drops**: Honeycomb (shears) and a Honey Bottle (bucket) — see [Storage Blocks](storage-blocks.md). **Ink Sac** drops live from Squid (see [Combat & Mobs](combat-and-mobs.md)) and feeds Black Dye.
- **Future** (data planted ahead of a live source): Glow Berry. No way to get this today; `/give` only.

Press **B** in-game to scroll through them all — over 80 materials and counting.

## Stack sizes

Most blocks stack to **64**. Tools don't stack — one tool per slot. Food usually stacks to 64 too.

## Items

There are three kinds of items in your bag:

- **Blocks** — anything you can place in the world. Stone, wood, cobblestone, beds, campfires, village bells.
- **Tools** — pickaxe, axe, shovel, sword, hoe, bow, flint and steel. Each has a **durability bar** at the bottom of the slot.
- **Materials** — things that aren't blocks but aren't tools either. Sticks, iron ingots, coal, wheat, bones, raw meat.

## Tools and tiers

Tools come in five tiers, from cheapest to best:

| Tier | Mining level | Where to get it |
|---|---|---|
| **Wood** | Stone, coal | 3 planks + 2 sticks |
| **Stone** | + Iron ore | 3 cobblestone + 2 sticks |
| **Iron** | + Diamond ore | 3 iron ingots + 2 sticks |
| **Diamond** | + Satori (the gem) | 3 diamonds + 2 sticks |
| **Satori** | All | 3 Satori + 2 sticks (rare!) |

Higher-tier tools mine faster and last longer. **A wooden pickaxe can't mine iron — it'll break the block but drop nothing.** Always carry the right tool for what you're after.

The five tool shapes are:
- **Pickaxe** — for stone, ores, cobblestone.
- **Axe** — for wood, logs, planks. Also okay as a weapon.
- **Shovel** — for dirt, sand, gravel, snow.
- **Sword** — best weapon; useless on blocks.
- **Hoe** — for tilling grass into farmland. See **[Farming](farming.md)**.

There's also:
- **Bow** — ranged weapon. Needs arrows (stick + feather recipe). Right-click to fire.
- **Flint and Steel** — lights campfires instantly. See **[Food & Cooking](food-and-cooking.md)**.

## Durability

Every tool has a durability number that ticks down as you use it. The **bar at the bottom of the tool's slot** shows the remaining durability — green when full, red when low.

When a tool drops below 10% durability you'll see a toast: *"Your Iron Pickaxe is almost broken — repair or replace soon."* That's your one warning.

When a tool runs out, it breaks and disappears. A second toast confirms it: *"Your Iron Pickaxe broke!"*

Tools don't break mid-swing on a block you've already cracked — the swing finishes first.

## Dropping items

Press **Q** to drop one of whatever is in your active hotbar slot. Tools drop whole (you can't drop "half a pickaxe"). Stacks drop one item at a time.

Dropped items become a small floating cube that other players can walk into to pick up. **For you, the dropper, there's a 1.5-second pickup delay** — so you can't accidentally instantly hoover an item you meant to give away. **Other players grab it instantly.**

Drops despawn after 5 minutes.

## When the bag is full

If you try to pick up something and your bag is full, you don't pick it up. Some quest rewards drop at the villager's feet if your bag is full — the toast tells you.

A common move is to **Q-drop the things you don't need** before you accept a quest reward you do.

## Tips

- Tools you mostly use (pickaxe, sword, food) belong in slots 1–3. Less-used (shovel, axe, blocks-to-place) in 4–9.
- A second tool of the same type in your bag is a backup. When the active one breaks, the second one isn't auto-equipped — you'll need to pick it up manually.
- If you find yourself drowning in cobblestone, place some down as walls or floors, or stash the overflow in a **Chest** — see [Storage Blocks](storage-blocks.md) for the five chest tiers.
