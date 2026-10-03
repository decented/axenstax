# Armour

Armour is your second health bar. Without it every hit lands on your HP directly; with it, a chunk of each hit gets soaked up before HP ticks down.

> **Status:** Armour is **live**. You craft it, equip it in dedicated inventory slots, and it reduces the damage you take in combat. Durability wears down on hit and a broken piece auto-unequips. A small armour-points readout shows by your hearts.

## Four slots, five tiers

Your inventory has **four dedicated armour slots** — one per piece. Open your inventory and **click** a piece onto its slot to equip it (click an equipped piece to lift it back out). Wrong-slot armour just bounces back, so you can't lose a piece on a stray click.

- **Helmet** — head protection
- **Chestplate** — torso (takes the most hits)
- **Leggings** — legs
- **Boots** — feet

Each piece can be made of:

- **Leather** — cheap starter armour
- **Iron** — solid mid-tier
- **Diamond** — high tier
- **Satori** — top tier (the orange Bitcoin gem)

> **Chainmail is back — drop only.** It **can't be crafted**. The only way to get it is to kill **Marauders**, who drop a random Chainmail piece (helmet, chestplate, leggings, or boots) about 1 in 10 kills. Same durability as Iron, but Chainmail is actually the **weakest** tier by armour points — every slot is equal to or lower than the matching Iron slot (12 points total vs Iron's 15, Diamond's 20). It's a lore/cosmetic drop-only tier, not a mid-tier upgrade — grind a stack of Marauders if you want the look, not for the protection.

## How damage reduction works

Every armour piece has **armour points** based on slot + material. Add up your equipped points and the formula is:

```
damage taken = raw damage × (1 − reduction%)
reduction%   = min(armour_points × 4%, 80%)
```

So **4% reduction per point, capped at 80%.** Even a full Satori set leaves 20% of every hit getting through — no godmode tier.

### Quick reference

| Set | Total points | Damage reduction |
|---|---|---|
| Bare | 0 | 0% |
| Full Leather | 7 | 28% |
| Full Iron | 15 | **60%** |
| Full Diamond | 20 | 80% (capped) |
| Full Satori | 24 | 80% (capped) |

Diamond and Satori both hit the cap on damage reduction. What you get extra from Satori is **durability** — your pieces last 2× longer.

## Per-slot point values

| Slot | Leather | Iron | Diamond | Satori |
|---|---|---|---|---|
| Helmet | 1 | 2 | 3 | 4 |
| Chestplate | 3 | 6 | 8 | 9 |
| Leggings | 2 | 5 | 6 | 7 |
| Boots | 1 | 2 | 3 | 4 |

**The chestplate is always the strongest slot.** That's because the chest takes the most hits in real play — the maths follows the body.

## Durability

Each piece has its own durability bar. Chestplates last the longest (they're the tankiest); Helmets and Boots wear faster. Durability roughly **doubles** as you climb tiers: a Diamond Chestplate lasts about 2× as long as an Iron Chestplate.

When a piece hits 0 durability it **breaks** and auto-unequips. Broken armour contributes 0 points to your total reduction — bring a spare.

## Crafting recipes

Each piece is the matching material in a body-shaped pattern on the crafting grid:

- **Helmet** — 5 of the material in a top-arc (covers your head)
- **Chestplate** — 7 of the material in a vest shape, with a gap at the neck
- **Leggings** — 7 of the material in a ∏-shape (upside-down U)
- **Boots** — 4 of the material in two parallel columns

(Chainmail has no recipe — it's drop-only from Marauders. See the note at the top of this page.)

> **Rubber Boots** are a special case — only **boots** can be made of rubber, and they boost your sprint speed by 40% (1.4×). See [Crafting](crafting.md) for the grid diagrams.

## Coming soon

> **Coming soon:** a fuller HUD armour bar. Right now equipped armour shows as a small **points readout** next to your hearts; a row of chest-shaped icons is planned to replace it.
