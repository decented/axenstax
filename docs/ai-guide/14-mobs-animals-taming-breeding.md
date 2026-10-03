<!-- SOURCE: game/engine/src/{mob.rs, species_ai.rs, mob_ai.rs, companion.rs, tameable.rs, breeding.rs, genetics.rs, wolf.rs, nostrich.rs, bear_ai.rs, hyena_ai.rs, squid_ai.rs, animal_products.rs, bee_hive.rs, game_loop.rs, save.rs, entity.rs} + docs/ai-guide/99-accuracy-and-deferred.md | Verified against code 2026-06-22 -->

# 14 — Mobs, Animals, Taming & Breeding

Purpose: the verified reference for every creature in the world — what it does, which ones you can tame, which you can breed, what you can ride, and what they drop. If a creature or action is not on this page, treat it as not in the game.

> Accuracy rule (from page 99): only the tame/breed/ride/harvest actions listed as **working** below are real. Where a creature is "defined but its special behaviour isn't switched on", this page says so plainly. Never promise a deferred action.

---

## 1. The roster (29 creatures)

The game defines **29 creatures** (`mob.rs::MobType`). Each is **passive** (won't attack you on its own) or **hostile** (will). A handful of "passive" animals will still defend themselves or run when you hit them — that's noted per creature.

### 1.1 Passive / neutral animals

| Creature | What it does (verified behaviour) | Notable |
|---|---|---|
| **Cow** | Wanders. Runs away if you hit it. | Milk it with a bucket; drops beef + leather |
| **Pig** | Wanders. Runs away if you hit it. | Drops porkchop |
| **Sheep** | Wanders. Runs away if you hit it. | Shear it for wool; drops mutton + wool |
| **Chicken** | Wanders. Runs away if you hit it. Lays eggs over time. | Lays an egg roughly every 10 in-game minutes; drops feathers + raw chicken |
| **Rabbit** | Small and skittish — **hops** about and **bolts fast** when you get close. | Very fragile (3 health); drops raw rabbit, sometimes a hide |
| **Goat** | Wanders the mountains and occasionally **charges and rams** a nearby player or animal for a little knockback. | Watch for the lowered head; drops 0–2 wool |
| **Horse** | Wanders in herds. Runs away if you hit it. **You can ride it.** | Faster than other animals; drops leather only (it's a mount, not food) |
| **Donkey** | Wanders (Plains/Savanna). Runs away if you hit it. **You can ride it.** | Drops leather; a mount, not food |
| **Mule** | Wanders (rare). Runs away if you hit it. **You can ride it.** | Drops leather; a mount, not food |
| **Nostrich** | A purple ostrich (the world's mascot). Lives on the savanna in little family groups. Flees from danger but **kicks back** if you corner it. **You can tame it.** | Tamed ones lay a special egg and shed feathers over time. **Hurting one is a big deal** — see §2.3 |
| **Bee** | Flies and drifts around pollinating crops. Peaceful **unless you hit it** — then it gets angry and **stings** (and the bee dies after stinging). | Drops a stinger. Honey is **not** obtainable yet — see Deferred |
| **Squid** | Drifts gently in water. Can't survive on land. | Drops ink sacs |
| **Fish** | Drifts about in the water and can't survive on land (like the squid). | Drops raw fish; you can also catch fish by fishing |
| **Glow Squid** | A glowing deep-water/cave squid; drifts like a normal squid. | Drops glow ink |
| **Fox** | A forest/taiga animal. Runs away if you hit it. **You can tame it** (with berries). | Drops a pelt (leather) |
| **Reindeer** | A tundra herd animal. Peaceful wanderer; flees when hit. | Drops leather + venison |
| **Villager** | Lives in villages. Right-click to talk / open quests. Doesn't fight. | Drops nothing (please don't hurt them — it's pointless and the village will remember) |
| **Peddler** | A rare wandering drifter that walks to a village, claims a home, and settles in as a villager. | Drops nothing |
| **Knight** | A village **defender** — won't hurt you, but guards the village and fights off raiders. Tanky (60 health). | Appears when a village is big enough; drops iron |

> The **Wolf** sits between the two lists. At the data level it's **passive** (a wild wolf won't chase or attack you on its own — wolf-combat AI isn't wired), so it behaves like a wary neutral animal until you tame it. It's listed once below, beside the true hostiles, because it's the only mob you tame with a bone. See §2.1.

### 1.2 Hostile creatures

| Creature | What it does (verified behaviour) | Notable |
|---|---|---|
| **Wolf** (wild) | A wary wild wolf — it **won't attack you unprovoked** (it's passive in the data). **You can tame it with a bone** to make it a companion that follows you. | Untamed ones drop leather + bone |
| **Bear** | A big forest/taiga omnivore. **Smells nearby crops and food chests** (within ~12 blocks), walks over, and eats/raids them. Charges if you hit it. | Tanky (30 health); drops bone + leather |
| **Hyena** | A savanna pack hunter. **Lazy by day, hunts at night.** A pack of 3+ close together gets a damage boost. | Fast; drops bone |
| **Shark** | The ocean's apex predator in deep water. **Only hunts players who are themselves in the water** — get out of the water (onto the shore or any dry block) and you're safe. It's telegraphed and avoidable. | Drips a shark tooth while hunting (you don't have to kill it) |
| **Polar Bear** | The tundra's apex threat. Chases you. | Drops bone + leather |
| **Brigand** | A human bandit. Patrols around its hideout by day, chases you at night, flees home when badly hurt. | 16 health; drops bits of wool/bread/iron |
| **Marauder** | A tougher human raider. Never flees. | 28 health; drops iron, and occasionally a piece of chainmail armour |
| **Berserker** | The boss of a rare brigand hideout. **Always aggressive, day or night.** Never flees. | 45 health; always drops a trophy + iron |

---

## 2. Taming — which animals become yours, and how

**Five creatures can be tamed** — Wolf, Cat, Parrot, Fox, and Nostrich (verified in the right-click handlers in `game_loop.rs` + the taming code). Each needs the right food in your hand; you right-click the animal with it. Taming is a **chance per feed** (about 1 in 3), so it usually takes a few tries — and each try uses up one food item, even a miss. That's normal; keep going.

| Animal | Tame it with | What you get once it's tamed |
|---|---|---|
| **Wolf** | **Bone** | A companion that **follows you** |
| **Cat** | **Raw fish** | A companion that **follows you** |
| **Parrot** | **Wheat seeds** | A companion that **follows you** |
| **Fox** | **Berries** | A companion that **follows you** |
| **Nostrich** | **Berries** | A companion that **follows you** and can **sit/stay** (right-click with an empty hand to toggle). A tamed Nostrich also **lays a special egg** and **sheds a feather** every so often |

Notes verified from code:
- A tamed companion **stays close** to its owner and stops when it's near enough — it won't get dragged across the world if you sprint or teleport far away.
- Once tamed, a pet is **no longer wild** — it won't vanish when the area unloads, and it's saved with your world (see §6).

### 2.1 The Wolf is special
The moment you tame a wild wolf with a **bone**, it flips from wary to a friendly follower that **follows you** around. A **tamed** wolf gives you nothing if it dies — you tamed a friend, you don't harvest it. (Deeper wolf commands — a sit/stay toggle, defending you in a fight — are designed but **not** wired up yet; for now a tamed wolf just follows. See Deferred.)

### 2.2 What "tameable" does NOT include
Cows, pigs, sheep, chickens, horses, goats, rabbits, squid, fish, bears, etc. are **not tameable** — there's no "make it mine" for them. (You can still **breed** the farm animals and **ride** a horse, donkey or mule — different systems, below.)

### 2.3 Be kind to the Nostrich
Killing a Nostrich triggers a long in-world penalty called **"The Nostrich's Vow"** — it blocks trading and good standing for a while. The game is deliberately built so that **looking after creatures pays off better than hurting them**. If a player wants Nostrich eggs and feathers, the answer is: **tame one and let it lay** — never hunt it.

---

## 3. Breeding & genetics — growing a herd

You can breed the **core farm animals** by feeding two adults of the same kind the right food. Feed one, feed another nearby, and they pair up into a baby; the baby grows into an adult after a while. (Verified in `breeding.rs` + the feed handler in `game_loop.rs`.)

| Animal | Breed it with |
|---|---|
| **Cow** | Wheat |
| **Sheep** | Wheat |
| **Goat** | Wheat |
| **Pig** | Carrot |
| **Rabbit** | Carrot |
| **Chicken** | Wheat seeds |

How it works (verified):
- Feed an adult its food → it goes into "love mode" for about 30 seconds.
- Two in-love animals of the same kind close together (within ~8 blocks) → **one baby** appears between them.
- Both parents then go on a **cooldown** (a few minutes) before they can breed again, so you can't spam a population explosion.
- **Babies can't breed** until they grow up (about 10 in-game minutes). A baby is a smaller version of the adult.
- **Different kinds don't pair** — a cow and a pig won't make anything.

### 3.1 Genetics (the depth layer)
Breedable animals carry hidden **genes** (`genetics.rs`): size, a subtle coat tint, a "yield" quality (how much they give you), and a speed quality. A baby inherits each gene from **one of its two parents**, plus a tiny random nudge. So if you keep breeding your **best** animals together, the herd slowly improves over generations — that's real, verified behaviour, not flavour.

A concrete payoff that's actually wired up: shearing a sheep gives **more wool** if it's from a high-yield bred line (a champion sheep can give up to ~6 wool, an ordinary one ~2–4).

### 3.2 What you can't breed
Wolves, Nostriches, Horses, Donkeys and Mules are **not** bred with food (they're excluded on purpose — wolves and Nostriches are tamed, and the horse/donkey/mule are mounts). There's **no** donkey-plus-horse-makes-a-mule cross-breeding yet either. Hostile creatures, villagers, fish, squid, etc. can't be bred at all.

---

## 4. Mounts — riding

**Horses, donkeys and mules all work as mounts.** Right-click one to climb on; then WASD to ride, Space to jump, and Sneak to get off. While you ride, the animal stops wandering on its own and follows your steering. (Verified in `game_loop.rs` mount handler + steering, both gated on `is_rideable`, which covers Horse, Donkey and Mule.)

| Animal | Can you ride it? |
|---|---|
| **Horse** | **Yes** — fully working |
| **Donkey** | **Yes** — same mount handler as the horse |
| **Mule** | **Yes** — same mount handler as the horse |

What's **not** in yet: carrying cargo (a chest on a donkey/mule) — that's a follow-up. Riding and steering all three works now.

---

## 5. Drops & animal products

What creatures give you (verified `mob.rs::drops_for` + the product handlers):

- **Cow** → raw beef (+ leather, sometimes a bone). **Milk:** right-click a cow with a **bucket** to get a milk bucket. You can't milk the same cow again straight away — it needs a few minutes.
- **Pig** → raw porkchop (+ sometimes a bone).
- **Sheep** → wool + raw mutton. **Shearing:** right-click a sheep with **shears** for wool (and it grows back after a cooldown). A well-bred sheep gives more.
- **Chicken** → raw chicken + feathers, and **lays eggs** on the ground over time (about every 10 in-game minutes).
- **Rabbit** → raw rabbit (and sometimes a hide). **Horse / Donkey / Mule / Fox / Reindeer** → leather (Reindeer also venison).
- **Squid** → ink sacs. **Glow Squid** → glow ink. **Fish** → raw fish.
- **Shark** → drips a **shark tooth while it hunts** (you don't have to kill it).
- **Bear / Polar Bear** → bone + leather. **Hyena** → bone. **Wolf** (untamed) → leather + bone.
- **Brigand / Marauder / Berserker / Knight** → iron and bits of loot; the **Berserker** always drops a trophy.
- **Villagers, Peddlers, tamed Cats** → nothing. **Parrots** → the odd feather.

---

## 6. Persistence — what's still there after you save and load

(Verified in `save.rs::tamed_mobs_to_saved`.)

| Creature | Survives save/load? |
|---|---|
| **Tamed Wolf** | **Yes** |
| **Tamed Nostrich** | **Yes** (keeps its egg/feather timers) |
| **Tamed Cat / Parrot / Fox** | **Yes** |
| **Horse / Donkey / Mule** | **No** — these are wild fauna with **no owner**. Riding state lives on the player, like a borrowed mount, so a mount you rode won't be waiting where you left it after a reload. |
| **Untamed wild animals** | Not individually saved — the world re-populates with fresh wildlife on load. |

So: **tame a pet and it's yours for keeps.** A horse, donkey or mule is great to ride right now, but don't expect "my mount" to be a permanent, saved companion the way a tamed wolf is.

---

## Deferred / not yet (do NOT present these as available)

- **Donkey & mule cargo-carrying.** You *can* ride donkeys and mules (the mount handler is shared with the horse via `is_rideable`) — what's **not** wired is carrying cargo, i.e. strapping a chest to a donkey/mule to haul items. Don't promise pack/cargo mounts. (Riding + steering verified live in `game_loop.rs`; no chest-carry inventory exists for them.)
- **Honey.** Bees pollinate and can **sting**, and a Bee Hive block exists — but **honey is not obtainable**. The hive doesn't fill up (the fill behaviour isn't wired), and no honey-harvest interaction runs in the game loop. **Don't tell a player they can collect honey or honeycomb.** Bees are still fun to watch and will sting if provoked.
- **Mount ownership / saved mounts.** There's no "this horse/donkey/mule is mine" that persists across save/load — mounts are wild fauna with no owner, so one you rode won't be waiting where you left it after a reload. Don't describe a mount as a saved pet.
- **Pet extras** (threat-alarm, parrot flight, a tamed wolf fighting hostiles for you, deeper commands like a pet bed or recall) are follow-up polish and **not** in yet. The verified behaviour for a tamed Wolf/Cat/Parrot/Fox is simply: it **follows you**. Only the **Nostrich** has a wired **sit/stay** toggle (empty-hand right-click); the wolf's sit/stay and combat-assist are written in code but not yet hooked into gameplay, so don't promise them.
- **Predator-prey food chain** (foxes hunting rabbits/chickens, sharks thinning a fish school, fish fleeing predators or panicking) is **not** wired. Fish and squid just **drift** — they don't react to players or predators, and there's no school-panic. Don't promise a working food chain beyond what *is* listed: the shark hunts players who are in the water, bees sting when hit, bears raid crops/food-chests, hyenas hunt at night.

When a player asks for one of these, be honest and kind, then offer what *does* work: "You can't strap a chest to a donkey yet — but you *can* hop on and ride one, just like a horse. Want to find one?"
