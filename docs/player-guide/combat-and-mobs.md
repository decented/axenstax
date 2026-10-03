# Combat & Mobs

You'll meet mobs — moving creatures — almost as soon as you spawn. Some wander past and ignore you, some give you food and materials when you swing at them, and some are out for blood.

This page is the **full mob roster** plus how combat works. Want a lesson instead of a list? Try [Surviving your first night](https://learn.axenstax.com/docs/journey/learn-journey/surviving-your-first-night.md).

## How attacks work

**Left-click** while pointing at a mob within reach (about 4 blocks). You swing. The mob takes damage based on what you're holding:

| Weapon | Damage |
|---|---|
| Bare fist | 1 |
| Wooden sword | 4 |
| Stone sword | 5 |
| Iron sword | 6 |
| Diamond sword | 7 |
| Satori sword | 8 |

Axes actually hit **harder** than swords at most tiers (7 damage at Wood/Stone/Iron; 9 at Diamond; 10 at Satori) — they're slower in feel but a strong fallback weapon. Pickaxes, shovels, and hoes do 1 damage — they're tools, not weapons. Bows aren't built for melee: swung by hand they do 1.0 damage at Wood/Stone, 1.5 at Iron, 2.0 at Diamond, and 2.5 at Satori (they're meant for ranged use).

### Critical hits

If you swing **while in the air** (after a jump), the hit is a **critical** — about 1.5× the normal damage. Combine jump + sprint for a big-damage opener.

### Knockback

Every hit pushes the mob slightly away. Sprinting + attack = bigger knockback. Useful for keeping distance from a charging **Brigand**.

### Cooldown

After each swing you have a brief cooldown (~0.5 s) before the next swing lands. Spam-clicking doesn't help — wait for the swing to land before the next click.

## Health + hunger

- You have **10 hearts** (20 health points). Most hits take half a heart to a heart and a half.
- You have **20 hunger points** that drain as you play.
- When **hunger ≥ 18**, health regenerates over time. Below 18, no regen.
- When **hunger = 0**, hearts start dropping. Eat something.

See [Food & Cooking](food-and-cooking.md) for what to eat and how much it heals.

## Death + respawn

When your hearts hit zero you die. The screen goes red and a **Respawn** button appears. Click it (or wait — there's an auto-respawn timer).

- In **Survival**, your full inventory (hotbar + bag, all 36 slots) is snapshotted into a **Grave** block placed at or near the death spot. A toast tells you the exact coordinates: *"Your grave is at x, y, z — go and reclaim it."*
- The Grave does **not** despawn or time out — it sits there until you (or anyone) mines/opens it to recover the items. Nothing is lost by taking your time getting back.
- If there's genuinely no safe air pocket to place a Grave in (rare), the game falls back to the old behaviour: items scatter as floating pickups at the death spot and despawn after **5 minutes**. This is a fallback, not the normal case.
- A death marker also drops on the map so you can find your way back either way.
- You respawn at your **spawn point** (your saved spawn, or world origin if no bed is set).

In **Creative** you can't die — health stays full, you don't drop inventory.

---

## The mob roster

There are **30 mobs** in Axe'n'Stax. Here's an honest thing worth knowing: almost all of them are fully alive — wandering, fleeing, breeding, fighting, or trading exactly as described below. A couple of specific behaviours are still mid-build, and where that's true you'll see a `> **Coming soon:**` note calling out exactly what's missing. Everything outside those notes is what really happens right now. ✅

Mobs come in two kinds under the hood: **passive** (won't start a fight) and **hostile** (will). Right now:

- **Passive** mobs wander and idle. Most of them **flee** when you hit them — Cow, Chicken, Pig, Sheep, Horse, Rabbit, Goat, Squid, Nostrich, Fish, Glow Squid, Fox, Reindeer, Donkey, Mule, and Crab all bolt on the first hit. The exceptions that stand their ground are Wolf, Bee, Villager, Peddler, Knight, Cat, and Parrot.
- **Hostile** mobs wander, then **chase** you when you get within their detection range.

Where mobs come from:

- **Daytime scatter** — passive livestock and wild animals appear as the world generates, weighted by biome.
- **Night spawner** — needs night + darkness, and only ever produces **Brigand**, **Marauder**, **Bear**, and **Hyena**.
- **Structure spawns** — **Villagers**, **Knights**, and **Peddlers** come from villages; **Brigands**, **Marauders**, and **Berserkers** come from Brigand Hideouts.

---

## Passive livestock

Your friendly farm animals. They wander grassy biomes, never flee, and are your everyday source of food, leather, wool, feathers — and **bones** (livestock are the bone source now, which feeds the bonemeal and farming path). See [Food & Cooking](food-and-cooking.md).

| Mob | Colour | HP | Where to find it | Drops |
|---|---|---|---|---|
| **Cow** | brown | 10 | Plains, Forest, Birch Forest, Savanna | 1–3 **Raw Beef** (always), 0–2 **Leather**, ~12.5% 1 **Bone** |
| **Pig** | pink | 10 | Plains, Forest, Jungle | 1–3 **Raw Porkchop**, ~12.5% 1 **Bone** |
| **Sheep** | white | 8 | Plains, Forest, Birch Forest, Taiga, Snowy Tundra, Savanna, Mountains | 1 **Wool** (always), 1–2 **Raw Mutton**, ~12.5% 1 **Bone** |
| **Chicken** | white (small) | 4 | Plains, Forest, Birch Forest, Taiga, Savanna, Jungle | 1 **Raw Chicken** (always), 0–2 **Feather** |

The **Cow** is the most reliable starter meal and the only easy early **Leather**. The **Chicken** is tiny and fragile (4 HP) — one swing usually does it.

---

## Passive wild animals

These all **spawn and roam their home biomes right now**, and most of them have real, live AI beyond just wandering — hopping, fleeing, charging, flying, breeding. Their drops still work too.

| Mob | Colour | HP | Where to find it | Drops |
|---|---|---|---|---|
| **Horse** | brown | 30 (fast) | Plains, **Savanna** (common), Forest | 0–2 **Leather** |
| **Donkey** | grey-brown | 25 | Plains, Savanna | 0–2 **Leather** |
| **Mule** | dark brown | 28 (fast) | Plains (rare in the wild — mostly a bred Donkey×Horse cross) | 0–2 **Leather** |
| **Rabbit** | tan | 3 (fragile, fast) | Almost everywhere — Plains, Forest, Birch Forest, Taiga, Snowy Tundra, Jungle, **Desert**, Mountains | 1 **Raw Rabbit** (always), ~15% **Rabbit Hide** |
| **Goat** | off-white | 10 | **Mountains** (common) | 0–2 **Wool** |
| **Bee** | yellow | 10 | Plains, Forest | 1 **Bee Stinger** |
| **Fox** | orange | 8 (fast) | Forest, Taiga | 0–1 **Leather** (pelt) |
| **Reindeer** | brown | 14 | Taiga, **Snowy Tundra** (common) | 0–2 **Leather**, 1–2 **Raw Beef** (venison) |
| **Cat** | grey-brown | 8 | Jungle | Nothing — it's a companion, not a resource |
| **Parrot** | red | 6 | Jungle | 0–2 **Feather** |

**Riding is live and needs no saddle** — there is no saddle item in the game. Right-click any rideable mob (**Horse**, **Donkey**, **Mule**) to mount it instantly. Your **first mount is what tames/keeps** that animal as yours. **Breeding is live too**: feed two adults while sneaking (the sneak-gate keeps breed-feeding and mounting from colliding), and a **Horse × Donkey** pair can produce a **Mule**.

The **Rabbit** hops and flees, dispatched every tick — it's genuinely skittish now, not just decorative. The **Goat** charges: get too close and it rams you for **1.0 HP damage** plus real knockback. The **Bee** actually flies, returns to its hive, and produces honey from nearby flowers (proximity-based today, not a full pollinate-and-return loop) — and it stings back for real, dying after landing the hit (Minecraft-parity — a bee only gets one sting). **Cat**, **Parrot**, and **Fox** are tameable companions built on the same framework as Wolves (Cat via **Raw Fish** or a guaranteed-tame **Cat Treat**, Parrot via **Wheat Seeds**, Fox via **Berries**) — right-click one with an empty hand once tamed to cycle its command through **Follow / Stay / Wander** (Parrot also gets **Perch**, hopping to your shoulder).

The **Rabbit** is the most widespread mob in the whole game — you'll find one in nearly every biome, including the **Desert** where almost nothing else lives.

---

## Ocean & shoreline animals

Water-bound mobs, plus one shoreline scavenger.

| Mob | Colour | HP | Where to find it | Drops |
|---|---|---|---|---|
| **Squid** | blue | 10 | **Ocean** (in the water) | 1–3 **Ink Sac** |
| **Fish** | pale blue-grey | 3 (fragile) | **Ocean** — schools together, flees predators + players | 1 **Raw Fish** (always) |
| **Glow Squid** | glowing teal | 10 | **Ocean**, deep water/caves | 1–3 **Glow Ink** |
| **Crab** | red-orange | 6 | Sandy shoreline near sea level, any coastal biome | 1–2 **Crab Claw** |

**Squid**, **Fish**, and **Glow Squid** all drift in open water live, and will genuinely **suffocate for real damage** if they end up stranded on land or out of water — don't fish one out and leave it flopping. **Fish** school together and panic as a group: spook one and the whole shoal bolts. The **Crab** wanders the shoreline (sand near sea level only) and its claw is the input material for the **Reach Claw** build tool.

---

## Hostile wild animals

Wild predators. These **chase you for real** when you get close enough.

| Mob | Colour | HP | Where to find it | Drops |
|---|---|---|---|---|
| **Bear** | dark brown | 30 (tanky) | Forest, Taiga + the night spawner | 1–3 **Bone**, 0–1 **Leather** |
| **Hyena** | tan | 12 (fragile, **very** fast) | Savanna + the night spawner (on sand) | 1–2 **Bone** |
| **Polar Bear** | white | 35 (tanky) | **Snowy Tundra** | 1–3 **Bone**, 0–1 **Leather** |
| **Shark** | slate grey | 30 | Deep **Ocean** (rare apex predator) | 1–2 **Shark Tooth** |

**Bears** are slow to kill and hit hard — keep your distance and use knockback, or shoot from range. A **Bear** drops good bone and the odd hide of leather. Bears can genuinely **smell food**: a hungry Bear will walk to a nearby mature crop and eat it (reverting the block back to tilled soil), or break into an unguarded chest and steal a single food item. Keep your farm and stores out of Bear range, or fence them.

**Hyenas** spawn in **packs of 2–4** (the pack spawning is live), and they're **very** fast. A single one is weak, but a pack can swarm you. Put your back to a wall and thin them out one at a time. Hyenas also run a real **lazy-day / hunting-night cycle** — lazy and low-threat by day, actively hunting once night falls.

**Polar Bears** are the tundra's answer to the Bear — similar toughness and drops, further north in **Snowy Tundra**. **Sharks** are the ocean's apex predator: they hunt Fish and wounded prey in deep water, and drip a Shark Tooth when they land a hit on prey (not only on a kill) — telegraphed and avoidable if you stay out of deep water.

> **Coming soon:** the **Hyena**'s pack damage boost — a pack of 3+ still swarms you, it just doesn't hit any harder yet.

---

## Human raiders — the Brigand family

Bandits. They spawn at **night** and from **Brigand Hideout** structures, and their tier behaviour is fully live. Each tier is tougher than the last.

| Mob | Colour | HP | Detection | Behaviour | Drops |
|---|---|---|---|---|---|
| **Brigand** | brown | 16 | 16 blocks | Chases **only at night**; flees toward its hideout below 25% HP | 0–1 each of **Wool** / **Bread** / **Iron Ingot** |
| **Marauder** | grey | 28 | 24 blocks | Chases **only at night**; **never flees** | 1–2 **Iron Ingot**, 0–1 **Bread**, 0–1 **Leather**, ~10% a random **Chainmail** piece |
| **Berserker** | dark red | 45 | 32 blocks | **Always aggressive — day AND night**; never flees | 2–3 **Iron Ingot**, 1 **Brigand Chieftain Trophy** (always), 0–1 **Leather** |

The **Brigand** is the common raider — rush a lone one. The **Marauder** is the **only** source of **Chainmail** armour in the whole game (about a 1-in-10 chance of a single random piece per kill), so grind a few to kit out a set you can't craft any other way. The **Berserker** is boss-tier: it lives in a rare hideout, so you have to go hunt it out, and it'll come for you in broad daylight. Back off, pick your moment, and bring a good sword. Its **Brigand Chieftain Trophy** is guaranteed.

---

## Village NPCs

The folk of the villages. **None of them drop anything** — that's deliberate. They're worth far more alive (trades, quests, defence) than dead, and hurting them costs you.

| Mob | Colour | HP | Where to find it | Drops |
|---|---|---|---|---|
| **Villager** | brown robe | 20 (slow) | Villages | Nothing |
| **Peddler** | purple robe | 20 | A rare drifter, anywhere | Nothing |
| **Knight** | silver | 60 (tankiest) | Auto-spawns at a busy village | 2–3 **Iron Ingot**, 0–1 **Leather** |

**Villagers** claim a workstation to take a profession (**Farmer**, **Cook**, **Carpenter**, **Blacksmith**, **Builder**, **Miller**, **Baker**, or **Brewer**), and they have dialogue, quests, and a per-village reputation you can build up. A **Blacksmith** is made by placing a **Furnace** next to a villager; a **Builder** claims a **Drafting Table**. Hitting one warns you the first time; killing one **costs you reputation** with that village, which it remembers. See [Villages & Villagers](villages-and-villagers.md) and [Quests & Reputation](quests-and-reputation.md).

> **Coming soon:** the **Scribe** profession (it claims a Bookshelf, which hasn't shipped yet).

The **Peddler** is a rare wandering trader. Left alone, it walks to an unclaimed **Village Bell**, claims a dwelling, and **becomes a normal Villager** — that's how new villages grow. Don't kill it. See [Village Bell](village-bell.md).

The **Knight** is the village's defender (it replaced the old Iron Golem). It's the tankiest mob in the game at 60 HP, it's friendly to you, and it patrols the village attacking the nearest hostile inside its defend radius. It auto-spawns once a village has at least **5 houses** and **3 employed villagers** (roughly one Knight per 10 employed). Don't pick a fight with one unless you really mean it. See [Knights](iron-golems.md).

---

## Showcase mobs

The two star companions of Axe'n'Stax — both **fully live**, both with their own deep-dive pages.

### Nostrich

The Nostr mascot: a fast purple ostrich. It's the **fastest mob in the game**, lives in **Savanna only**, and spawns in family groups of 2–3. **HP 20.**

You tame a **Nostrich** by feeding it **Berries** (1-in-3 chance per feed). Once tamed it lays eggs near you, sheds feathers passively, and **sits or stands on command** (right-click with an empty hand). Tamed and wild ones both lay 1 **Nostrich Egg** every in-game day; tamed ones also shed 1 **Nostrich Feather** every 2 days. The lore is a Bitcoin nod: 1 Nostrich Egg is worth about **21** chicken eggs.

Don't pick a fight with it lightly. When hit, a **Nostrich** **kicks for 6 damage** and then flees. And there's **The Nostrich's Vow**: killing one, or eating its meat, triggers a roughly **20-minute curse** that blocks vendor trade, zeroes your village reputation, and suppresses sats payouts — you have to purify at a **Nostrich Memorial**. It still drops 1–3 **Feather** and 0–1 **Raw Nostrich Meat** on death, but the Vow penalty is rarely worth it.

Taming, the **Sit / Stand** toggle, eggs, feathers, and the walk-and-follow trail (a Follow-mode Nostrich physically pads after you across the map) are all live.

Full guide: [Nostrich](nostrich.md).

### Wolf

A grey companion, **HP 20**, found in Forest, Birch Forest, **Taiga** (common), and Snowy Tundra. You tame one by feeding it **Bone** (33% chance per click — the bone is used up even on a failed try, so bring a few).

Untamed wolves drop 1 **Leather** + 1–2 **Bone**. A **tamed** wolf drops **nothing** — killing your own wolf is pure loss. Taming, the **Sit / Stand** toggle, and full companion movement all work today: a standing tamed wolf physically **follows** you (it walks toward you past ~8 blocks, stopping short so it doesn't bounce at your feet), **joins your fights** when you attack a hostile, and **avenges** you — chasing down whatever just hit you — landing real contact damage.

Full guide: [Wolves](wolves.md).

---

## Tips

- A wooden sword + 2–3 cooked porkchops = enough kit to survive a first night.
- Stay on grass at night until you've got a sword and a torch-lit shelter — the night spawner only works in the dark.
- The campfire smoke pillar signals "human stuff here" and draws hostiles. Near a village, the **Knight** will handle them.
- **Bow + arrows** lets you fight **Bears** and the **Brigand** family safely from range. If you can craft them, do.

## See also

- [Food & Cooking](food-and-cooking.md)
- [Wolves](wolves.md)
- [Nostrich](nostrich.md)
- [Knights](iron-golems.md)
- [Villages & Villagers](villages-and-villagers.md)
- [Quests & Reputation](quests-and-reputation.md)
- [Village Bell](village-bell.md)
- [Surviving your first night](https://learn.axenstax.com/docs/journey/learn-journey/surviving-your-first-night.md)
