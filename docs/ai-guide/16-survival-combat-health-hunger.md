<!-- SOURCE: combat.rs, player_slot.rs, armour.rs, item.rs, slingshot.rs, crafting.rs, grave.rs, play_mode.rs, spawning.rs, mob.rs, game_loop.rs, data/mobs/*.toml, hud_ui.rs | Verified against code 2026-06-22 -->

# 16 — Survival, Combat, Health & Hunger

Purpose: the verified rules for staying alive — how health, hunger, healing, armour, weapons, day/night danger, death and game modes actually work in the current build. Numbers below are read from engine source; if a number isn't here, don't invent one.

> Reader note: these systems only apply in **Survival** mode. **Creative** players take no damage and never get hungry. **Adventure** and **Spectator** behave as described in the Game Modes section.

---

## 1. Health

| Fact | Value |
|------|-------|
| Player max health | **20 HP** (shown as **10 hearts**; 1 heart = 2 HP) |
| Starting health | full (20) |
| Death | health reaches 0 |
| Invincibility after a hit | **0.5 s** (10 ticks) — you can't be hit again until it ends |
| Damage flash | a short red flash when hurt |

### How you take damage (verified sources)
- **Hostile mobs touching you.** Any hostile mob within ~1.5 blocks deals **3 HP per hit** (baseline contact damage) and knocks you back. (`HOSTILE_MELEE_DAMAGE = 3.0`.)
- **Lava.** Standing in lava burns for **2 HP every half-second** (about 4 HP/sec). Flying modes (Creative/Spectator) are immune.
- **Bee sting, goat charge, shark bite.** Specific animals can hurt you: an angry bee stings, a provoked goat charges, a shark bites in deep water. (Exact numbers live in `species_ai.rs` / `goat_ai.rs`; the contact rule is the same i-frame system as above.)

### What does NOT hurt you (verified absent)
- **Falling does no damage.** There is no fall-damage code — you can drop from any height safely.
- **Drowning / suffocation does no damage to the player.** There is no breath/oxygen system for players.
- **Fire (as a block) does no standalone burn damage** — only lava deals environmental damage.

### How you heal
- **Eat food** — restores health directly (see Hunger below). This is the main way to recover.
- **Passive regeneration.** When your **hunger is high (18 or more out of 20)** and you're below full health, you slowly regenerate **+1 HP every 4 seconds**. Each regen tick costs a little hunger, so eating keeps it going.
- **Sleep in a bed** — restores full health (and skips the night). See Day/Night.
- **Respawn** after death restores full health.

Healing never works while you're dead — eat *before* you run out, not after.

---

## 2. Hunger

| Fact | Value |
|------|-------|
| Max hunger | **20** (shown as a hunger bar) |
| Starting hunger | full (20) |
| Idle drain | **−1 hunger every 30 seconds** (gentle — you don't have to chase food constantly) |
| Regen threshold | hunger **18+** lets passive health regen run |
| Starvation | when hunger hits **0**, you lose **1 HP every 4 seconds** |
| Starvation floor | starvation **cannot kill you** — it stops at half a heart |

**The loop:** hunger slowly drops over time and every time you passively heal. Eating refills it. Keep hunger topped up and you'll heal on your own; let it empty and you'll start losing health (but never die from hunger alone).

### Eating
Right-click while holding a food item to eat it. **Eating restores both health and hunger by the food's value.** You can only eat if you're hungry or hurt (it won't waste food at full). Eating has a short cooldown so one click doesn't eat a whole stack.

### Foods and their values (sample — higher = more restored)
| Food | Restores |
|------|----------|
| Potato (raw), Egg, Sugar Beet | 1 |
| Raw Chicken, Raw Mutton, Raw Fish, Corn, Cookie, Berries, Beetroot, Pumpkin | 2 |
| Raw Beef, Raw Pork, Carrot, Cream | 3 |
| Cooked Chicken, Cooked Mutton, Cooked Rabbit, Butter | 4 |
| Bread, Cooked Beef, Cooked Pork, Baked Potato, Cooked Rabbit | 5 |
| Cooked Fish, Cheese, Milk, Honey Bottle | 6 |
| Sweet Bread, Pancakes | 7 |
| Pumpkin Pie, Berry Pie | 8 |
| Stew, Loaded Baked Potato | 10 |
| Cake | 12 |

**Cooking is worth it:** cooked meat restores noticeably more than raw (e.g. Raw Beef 3 → Cooked Beef 5). Cook raw food on a campfire or in a furnace.

> Some high-value recipe foods (Cake, Pancakes, Beetroot Soup, etc.) have a hunger value defined but their crafting workstations aren't all built yet — see Deferred. Treat raw/cooked meats, bread, baked vegetables, and milk as the reliable everyday foods.

---

## 3. Armour

Armour reduces incoming damage. There are **four slots** and a tier ladder.

### Slots
**Helmet, Chestplate, Leggings, Boots.** Chestplate protects most, then Leggings, then Helmet/Boots. Each piece wears down independently.

### Tiers (materials)
From weakest to strongest: **Leather → Iron → Chainmail → Diamond → Satori.** (Chainmail sits between Iron and Diamond but has no crafting recipe and no live drop source right now — see Deferred. **Satori** is the top tier.) There are also **Rubber Boots** (boots only) — they don't add much protection but double your sprint speed.

### How protection works
- Each armour point cuts incoming damage by **4%**.
- Total reduction is **capped at 80%** — even a full Satori set still lets 20% of a hit through. There is no invincible armour.
- **Example: a full Iron set = 15 points = 60% reduction.** An 8-damage hit becomes 3.2 damage through full Iron.
- **A full Satori set hits the 80% cap** — that 8-damage hit becomes 1.6. Satori buys you the cap *plus* far more durability, not extra reduction past the cap.
- Every hit you take wears **1 durability off each equipped piece**. When a piece breaks (durability 0) it's removed automatically and stops protecting you.

The HUD shows a small **armour points** badge next to your hearts.

---

## 4. Combat

### Attacking
- Left-click / attack to swing at the entity in your crosshair.
- **Reach: 3 blocks.** You hit the closest thing roughly within 60° of where you're looking.
- **Attack cooldown: 0.5 s** (10 ticks) between swings.
- **Critical hits:** swinging while falling (in the air, not on the ground) does **1.5× damage**.
- **Sweep attack:** a swing also hits *other* mobs in the same forward arc for **40%** of the main damage — handy against a group.
- Hitting a prey animal (cow, pig, sheep, etc.) makes it **flee**. Hitting a neutral/hostile mob (bear, brigand) makes it fight back, not run.

### Melee weapon damage (per swing)
| Weapon | Wood | Stone | Iron | Diamond | Satori |
|--------|------|-------|------|---------|--------|
| **Sword** | 4 | 5 | 6 | 7 | 8 |
| **Axe** | 7 | 7 | 7 | 9 | 10 |
| Fist / other tools | 1 | 1 | 1 | 1 | 1 |

Note: axes actually hit **harder** than swords in this build (the sword's edge is its faster, cleaner swing feel; the axe trades that for raw damage).

### Ranged weapons
- **Bow.** Hold a Bow and have **Arrows** in your inventory, then fire. Arrow damage by bow tier: **Wood 5, Stone 6, Iron 7, Diamond 9, Satori 10.** (A bow used as a melee club only does ~1–2.5 damage — it's meant to be fired.)
- **Slingshot.** A simple wood-tier ranged tool that fires **Rubber Balls** in a lobbed arc. Damage scales with charge from **1 up to 3**. At half-charge or more it **stuns** passive/neutral animals for ~1 second — but carnivores (Bear, Hyena, Wolf) and the brigand family (Brigand, Marauder, Berserker, Knight) shrug it off.

> Combat feel is still being tuned. Targeting picks the nearest entity in your forward cone; precise aim-through-walls and finer melee feel are works in progress — don't promise pixel-perfect combat polish.

---

## 5. Day / Night Danger

The world has a day/night cycle. **Hostile mobs spawn at night**, in dark spots (no light) on solid ground — so torches and lit bases keep them away. Once you have a lit, walled home, nights are much safer.

### What spawns at night (verified pool)
- **Brigand** — the common night raider (~16 HP).
- **Marauder** — tougher brigand (~28 HP), never flees.
- **Bear** — in forest/taiga (~30 HP), aggressive.
- **Hyena** — in savanna, in **packs of 2–4** (~12 HP each).
- **Berserker** (~45 HP) and **Knight** (~60 HP) do **not** wander the dark — they're tied to **Brigand Hideouts**, a tougher place you go and find on purpose.

Wolves (~20 HP) roam forests/taiga/tundra by day and night. On **Peaceful** difficulty, hostile mobs won't attack you.

### Beds (set spawn + skip the night)
Right-click a **bed at night** to sleep. Sleeping:
1. **Skips to morning** (sets the time to just after sunrise).
2. **Restores your full health.**
3. **Sets your spawn point** to that bed — that's where you'll respawn if you die.

You can only sleep at **night** — clicking a bed in daytime just tells you it's not time to sleep.

---

## 6. Death

When your health hits 0 (Survival, non-Creative):
1. You die and a **death marker** is dropped on your map so you can find your way back.
2. **Your items go into a grave.** A `GRAVE` block is placed at a safe spot near where you fell (it searches for a nearby air cell — it never destroys your build and never lands in lava or the void). A toast tells you the grave's coordinates.
3. **Recover your stuff** by going back and breaking/opening the grave — items return to their **original inventory slots** where possible. Nothing is ever lost, charged, or destroyed on death.
4. **Respawn:** after about 2 seconds (or click the respawn button on the death screen) you respawn at your **spawn point** — your bed if you've slept in one, otherwise where you started the world — at **full health and full hunger**.

Special case: some worlds (e.g. blank-canvas / parkour worlds) have **keep-inventory** on — there you keep all your items on death, with no grave. Creative players are simply invulnerable and never die.

> Death never costs you money, score, or items — it only *moves* your items to a recoverable grave. This is deliberate.

---

## 7. Game Modes

One mode per world (chosen at world creation; the default is Survival).

| Mode | Can edit blocks? | Flies? | Takes damage / hunger? | What it's for |
|------|-----------------|--------|------------------------|---------------|
| **Survival** | Yes | No | **Yes** — full health/hunger/combat loop | The main game: mine, craft, fight, build, survive |
| **Creative** | Yes (instant break, infinite blocks) | Yes | **No** — invulnerable, no hunger | Building freely, designing, the Workshop |
| **Adventure** | **No** — world is read-only to you | No | Yes | Playing someone's map/challenge without wrecking it (doors, mobs, trading still work) |
| **Spectator** | No | Yes (noclip — pass through blocks) | No | Watching / exploring; can't touch anything |

Notes:
- In **Creative**, the hearts and hunger bars are hidden (nothing to track).
- **Adventure** v1 blocks *all* breaking and placing (a per-block allowlist is a planned follow-up).
- **Spectator** here is the local "noclip and watch" mode — the bigger networked spectator system (watching other players, entity-POV) is not in this build.

---

## Deferred / not yet (do NOT present these as available)

- **Fall damage, drowning, suffocation, and fire-block burns** — none of these hurt the player. Only lava deals environmental damage. Don't tell a player they'll get hurt falling or drowning.
- **Food poisoning / status effects.** The engine has a poison mechanic (poison drains health on a timer and can't kill you), but **no food or source currently triggers it** — every item's poison value is 0. Treat poison and all status effects (regeneration potions, strength, etc.) as **not in the game**. Don't warn a player that a food will poison them.
- **Food saturation depth.** The hunger bar shows your hunger *level* but not a hidden saturation buffer — don't explain a saturation mechanic as if it's surfaced.
- **Chainmail armour** exists as a tier in code but is **never craftable and has no drop source** right now. Don't tell a player how to get Chainmail.
- **Some high-tier cooked foods** (e.g. Cake, Pancakes, Beetroot Soup) have hunger values defined but their workstations/recipes aren't all built — they're gated out of quests. Steer players to bread, cooked meat, baked vegetables, milk, and similar reliable foods.
- **Combat targeting/feel is still being tuned** — don't over-promise precise aim or polished melee handling.
- **Per-player game modes.** Mode is set per *world*, not per player; you can't have one player in Creative and another in Survival in the same world.
