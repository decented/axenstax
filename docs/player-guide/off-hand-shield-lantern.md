# Off-hand, Shield & Lantern

You have two hands. The one you use for the selected hotbar slot is your **main hand**. The other one is your **off-hand**, and it can hold anything.

## The off-hand

- **Swap hands:** press **F**. The item in your main hand and the item in your off-hand trade places. (On a touch screen, tap the off-hand box.)
- **Where you see it:** a small box on the **left of your hotbar** shows the off-hand item. In the inventory screen the off-hand slot sits next to the boots, with a faint shield picture when it is empty. Click it like any other slot.
- **In your hand:** your off-hand item shows in your left hand, in first person and on other players.

### What the off-hand does

- **Right-click uses your main hand first.** If your main-hand item has nothing to do with what you are pointing at, your off-hand item gets the click instead.
  - Hold a **pickaxe**, keep a **torch** in the off-hand, and right-click a block: you put the torch down without putting the pickaxe away.
  - Hold a **sword**, keep **bread** in the off-hand, and right-click when you are hungry: you eat it.
- **Left-click always uses your main hand.** The off-hand never mines or attacks.
- **A bow or slingshot** takes its ammo from the off-hand first, then from your bag.
- **Light:** a torch or lantern in **either** hand lights up the dark round you.
- **If you are defeated,** the off-hand item stays with you, like armour. It does not go in your grave.

## The shield

Held in either hand, a shield stops hits that come from the front.

**Make one at a workbench** (3 wide, 3 tall), in a Y shape:

```
Shield (3×3 → ×1)
[P ][I ][P ]
[P ][P ][P ]
[  ][P ][  ]
P = Planks   I = Iron Ingot
```

**Use it:**

1. Hold the shield in either hand.
2. **Hold right-click.** Point at open air or plain ground. If you point at a chest, a door or a mob, the click does that job instead and the shield stays down.
3. It starts working after a quarter of a second. The shield lifts in front of you.
4. Let go to lower it.

**While it is up:**

- A hit from the **front** does no damage and does not push you back. You hear a clack.
- A hit from **behind** hurts as normal. So do lava, fire, falling and drowning.
- You walk slowly, like sneaking, and you cannot sprint.
- You **cannot mine or attack** until you lower it.

**Wear:** a new shield has **336** durability. Each block costs **1 plus the damage it stopped**, rounded down. A brigand hit of 3 costs 4. When it hits 0 the shield breaks.

## The lantern

A lantern is a light you can carry, stand up or hang.

**Make one at a workbench:** an iron ingot, a torch, an iron ingot, in a column. That makes 1 lantern.

```
Lantern (1×3 → ×1)
[I ]
[To]
[I ]
I = Iron Ingot   To = Torch
```

- **Stand it:** right-click the top of a block.
- **Hang it:** right-click the **underside** of a block, such as a cave ceiling.
- **Carry it:** hold it in either hand. It lights up the dark round you.
- A lantern gives light level **15**, a little brighter than a torch (14).
- Breaking a lantern gives the lantern back. It does not fall if the block it sits on goes away.

**Try it:** type `/give lantern` and `/give shield` in a test world.
