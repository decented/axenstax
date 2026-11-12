# Axe'n'Stax test missions: build 0.2.34

This is the test guide for the current build. A tester, or a chat assistant co-hosting their session, reads it for what to type, exact numbers, what success looks like, and why something might not work.

Everything in this file was checked against the game's code for build 0.2.34. If the game and this guide disagree, that is worth reporting.

- **Build:** 0.2.34 (desktop app, Linux)
- **This session:** first the "Fixed since last time" re-checks (Mission 0), then the four missions "On your own"
- **Coming next:** missions at a village, and missions with a friend

---

## Fixed since last time (re-check these first)

These were found in the last playtest (build 0.2.33) and are fixed in 0.2.34. Check each one and say whether it now works. Mission 0 below covers the first two.

1. **Torches look like torches.** Before, a placed torch was a dark brown block. Now it shows a yellow-orange flame on top of a brown wooden handle, by day and by night.
2. **Torchlight is bright at night.** Before, a torch at night barely lit the blocks around it (they showed about a seventh of their normal colour). Now the blocks next to a torch show nearly their full colour at midnight, about the same as at noon, in a warm light that fades over the next few blocks.
3. **Small blocks show the right picture.** A button shows its raised pad, a pressure plate shows its edge on all four sides, a lever's handle is wood-coloured, and a block in an item frame shows its whole picture on every side (a framed torch used to be a black cube).

**Other changes in 0.2.34** (nothing to re-check, but good to know):

- Rails and cables, powered ones too, no longer glow at night. They give off no light, so they now get as dark as the ground around them.
- Water next to a torch is lit by it at night.
- The Workshop is always daytime now, so working copies never go dark.

**Known, not fixed yet** (if one of these happens, it is already known):

- A torch in your hand, or dropped on the ground, still looks like a small dark cube with a torch picture on it. Only placed torches are fixed.
- The world is split into columns 16 by 16 blocks wide. A sealed room built across the edge of one of those columns can stay lit like daytime inside at night, even with no torch.

---

## Also new in 0.2.33 (the last playtest build)

- Villages that lost their villagers after a reload get villagers again.
- Crops and saplings are only checked in the chunks that have them, and a save only writes land that something has changed. Both should cut lag in big worlds.
- Birdsong and pollen only happen under open sky.
- Storms no longer arrive at the same minutes in every world.
- The Grey Quarry only finishes inside its own trial world.
- Water you pour drains properly after a reload, and lava no longer creeps further every time a world loads.
- A mineshaft chest and an emptied wild beehive stay the same when you walk away and come back.

From 0.2.32 (the last playtest):

- Birdsong plays less often (about every 60, 35 or 20 seconds, depending on how lively a place is), lower and softer.
- Storms are rarer (about 1 rain in 7 is a storm), and the lightning flash is gentler.
- Flying over land you changed no longer re-runs the world generator every tick (this was a lag cause).

---

## Before every mission (do this once)

1. **Make a new Survival world just for testing.** The missions use chat commands, and commands mark a world as "cheats used". Keep them out of your real world.
2. **Press T** (or /) to open chat and type a command.
3. **Type `/gm c`** to switch to Creative so you can fly. Find **flat, open grass with no trees**. Missions 1 to 3 all happen there.
4. **Type `/gm s`** to switch back to Survival. The game must reply **"Switched to survival"** (or "already in survival").
5. **Check the build number** at the bottom of the Settings menu. It should say **v0.2.34**. Use that number in the feedback.

**Good to know:** hunger drops 1 point (half an icon on the hunger bar) every 30 seconds, whatever you are doing. A full bar lasts about 10 minutes, so take some food (berries, bread or cooked meat), or use `/give bread 16` in a test world.

**Why step 4 matters:** worn paths and blight (missions 1 and 2) never happen in Creative. Creative also stops places from looking grey. If you can still fly, you are in Creative and those missions will not work.

---

## Mission 0: Torch at midnight (re-check)

**What it tests:** a torch shows its flame and lights the blocks around it at night.

1. In your test world, stand on open grass and type `/time set midnight` (the darkest time).
2. Type `/give torch 4` if you have no torches, and place one on the grass.
3. Step back two or three blocks and look at it.
4. Place the other three torches in a line, about 4 blocks apart, to light a path.

**Success:** the torch shows a yellow-orange flame on a brown handle. The grass right next to it is clearly lit in a warm light and you can see it is green; it gets darker over the next few blocks. Then type `/time set day` to go back to daytime.

**If it does not work:**
- The torch is a plain dark block: check the build number at the bottom of Settings says **v0.2.34**. An older build still has the bug.
- It is too dark to tell: try it inside a cave, or in a hole you dig, where there is no moonlight at all.

---

## Mission 1: Make a path

**What it tests:** grass that is walked over a lot turns into a worn dirt path.

**Do:**
- Pick a straight line of grass **9 blocks** long.
- **Walk along the line**, from one end to the other and back, **12 times**. Walking or running is fine; do not jump or fly.
- Walking or jumping on one spot never counts. You have to keep moving from block to block.
- Do it in one go without leaving the world.

**Look for:** after the 12th trip, **every block except the 2 ends** (the ones you turn round on) turns into a **Worn Path**. A Worn Path looks just like plain dirt, and breaking one drops dirt; that is how it is meant to be. The 2 end blocks, where you turn round, take twice as long (24 trips), so it is normal that they are still grass.

**The rules behind it:**
- A "footfall" is stepping onto a grass block from a different block, with your feet on the ground.
- A grass block that gets **24 footfalls** becomes a Worn Path.
- Standing still does not count. Walking or jumping on the spot does not count. Flying does not count.
- One trip there and back steps on each middle block twice, so 12 trips gives them 24 footfalls. You turn round on the end blocks, so they get only one footfall per trip.
- Only plain grass blocks wear (not dirt, sand or tall grass).
- Off in Creative.
- The game remembers partly-worn blocks only while the world is open. Leaving the world or restarting forgets the counts (a finished path block is saved).
- The guide character, Satoshi, may say something the first time a path appears.

**If it does not work:**
- Check you are in Survival (`/gm s`).
- Check you are walking on grass blocks, not dirt or tall grass.
- Do not jump while walking.
- Do all 12 trips without leaving the world.
- Only the 2 end blocks are still grass? That is normal (see "Look for").

---

## Mission 2: Grow a giant wheat field

**What it tests:** blight. A huge field of one crop is fragile. Some of it goes grey, and the grey spreads.

**Type:**
1. Stand on flat grass at one corner: `/we pos1`
2. Walk to the corner **16 blocks along and 16 blocks across**: `/we pos2`
3. Then: `/we set wheat_stage_0`

That plants a square field of wheat sprouts (about 17 by 17, nearly 300 plants) in one go.

**Do:** stay next to the field **in daylight** for **2 minutes** and watch. Then break one grey plant.

**Look for:**
- Within about a minute some wheat turns **grey** (a Blighted Crop).
- Grey keeps **spreading** to the wheat right next to it, even after the rest is fully grown.
- Breaking grey plants stops the spread from those plants.

**The rules behind it:**
- The game looks at land in **16 by 16 squares** (chunk columns). Blight only starts in a square that has **at least 64 crops**, with **90%** or more of them the same kind.
- A small field, or a field that falls across two squares with fewer than 64 crops in each, **never** starts a blight. That is why this mission plants a big field: some 16 by 16 square is sure to get at least 64.
- Crops get a growth turn every **10 seconds** (every 5 seconds with water within 4 blocks, or in rain).
- On each growth turn, each growing crop in such a field has a **1 in 40** chance to go grey instead of growing.
- A grey crop spreads to a same-kind crop beside it (same height) **1 in 8** times per growth turn.
- Crops need light: daylight, or torches nearby.
- Grey crops never grow, drop nothing, and bone meal does nothing to them.
- Blight is off in Creative and during trials.
- A grey crop counts as bare ground for how alive a place looks.

**If it does not work:**
- Check you are in Survival (`/gm s`). In Creative nothing goes grey.
- Check it is daytime (`/time set day`).
- Check the field is big (16 by 16 or more) and is all wheat.
- `/we size` tells you how big your selection is.

---

## Mission 3: Put up a Site Board

**What it tests:** the Site Board, a shared to-do list for a group building somewhere together.

**Type:** `/give site_board`

**Do:**
- Place the board.
- Right-click it. Add a job from the list, then tick that job done.
- Close the board and open it again.

**Look for:**
- The job and its tick are **still there** when you reopen it.
- The place round the board gets **a little brighter** when you tick a job.

**The rules behind it:**
- A board holds up to **8 jobs**.
- Jobs come from a fixed list: Plant trees, Build walls, Build a roof, Dig a well, Light the way, Make a garden, Bring food, Gather wood, Clear the ground, Build a bridge, Fence the animals, Decorate.
- There is no free typing on a board.
- Anyone can add, tick or remove a job.
- The game does not check that the job was really done; the tick is the group's word.
- Ticking a job done gives the place round the board a small, short lift, at most **once per board per game day**. Ticking and unticking again does nothing more.
- The crafting recipe: sticks down the left column, planks down the middle, sticks down the right column (3 high).

**If it does not work:** the brightening is small and only once per game day, so look for it on the first tick.

---

## Mission 4: The Grey Quarry

**What it tests:** a trial about a place, not a person. You bring a stripped grey quarry back to life.

**Type:** `/trial grey-quarry`
- This takes you to the quarry's own trial world, with a kit.
- It works even if the trial is still locked on the Trials board.
- `/trial off` leaves it.

**Do (in this order):**
1. Plant all **12 saplings** first (4 oak, 4 birch, 4 spruce) and put bone meal on them. Trees count most, and they take longest to grow.
2. Break the **2 cobblestones** at the top of the quarry to let the spring water run down.
3. Hoe the dirt rows and plant the seeds, carrots and potatoes.
4. Walk the **5 animals** (2 sheep, 1 cow, 2 rabbits) in from the fenced meadow with the Leads.

**Look for:**
- The task words change: **Grey and still → Something's stirring → Waking up → Alive**. There is never a number.
- Hold it at Alive for **20 seconds**.
- An end card with a **four-petal flower** (life, ideas, people, time) and one sentence about how you did it.

**The rules behind it:**
- **The kit:** 4 oak, 4 birch and 4 spruce saplings; 8 wheat seeds, 8 carrots, 8 potatoes; 8 bone meal; 16 bread; 8 wheat; 3 Leads; a bucket, a stone shovel and a wooden hoe; 1 Feast Table.
- The quarry is about 24 by 24 blocks, in four terraces.
- The quarry comes back to life **much faster** than a normal world (12 times faster). That is for this trial only.
- It stays daytime, and there are no hostile mobs.
- Water, animals and crops get the quarry a good way, but **trees carry it over the line**.
- The Feast Table in the kit does nothing here: a feast needs guests, and there are no villagers in the quarry.
- In a normal game, the trial unlocks the first time a place in your world goes grey, or after you finish the Power Plant trial. The command skips that.
- **Try again** resets the quarry so you can try a different way.

**If it does not work:**
- Plant the trees first. A quarry with crops and animals but no trees often stalls at "Waking up".

---

## How it all works (for questions)

### How alive a place is ("bloom")

- The game judges land in **16 by 16 squares**.
- Each square has a hidden liveliness. You never see a number. You see it as:
  - **Colour:** grey and washed out when low, warm green when high.
  - **Sound:** birdsong by day, crickets at night, in lively places under open sky.
  - **Life:** pollen by day, fireflies at night.
- **What makes a place lively:**
  - Living things covering the ground: grass, tall grass, flowers, crops, leaves and logs, saplings, water.
  - **Variety:** each different kind counts (each tree species, each crop or flower family, water, grass), and so does each different kind of animal standing there. Three kinds of tree beat twenty of one kind.
  - People: a place where more than one person has built, or where people keep coming back and sleeping, is livelier.
- **What makes a place grey:** bare, stripped or built-over ground, and grey (blighted) crops.
- **How fast it changes:**
  - A place brightens fairly quickly, most of the way in about 6 minutes.
  - It fades slowly, over about 40 minutes.
  - Places are only tracked within about 128 blocks of you.
- In Creative, places never look grey.

### Other Living World things (for later missions)

- **Feast:**
  - Place a Feast Table, hold 4 or more things to eat, and right-click it.
  - It needs guests: villagers within 32 blocks, or other players within 48.
  - The table becomes a Feast Marker and the place lifts, once per spot per game day.
  - With no guests it says "A feast needs guests. Bring friends, or find a village."
- **Commons Stone:**
  - Made from 8 cobblestone round a sapling.
  - Marks shared ground 16 blocks each way.
  - In Survival, only the person who built a block there can break it. In a game with a friend, their break just pops the block back.
  - Explosions, pistons, fire and falling blocks are not stopped.
- **Commons Shelf:** a shared chest. A shelf with something in every slot brightens the place; one that gets emptied makes it duller.
- **Villages:**
  - Rare: about one per 512 by 512 blocks, only on plains or forest.
  - No command makes villagers appear.
- The full player guide is in the `docs/player-guide` folder of this repository (see "Links" at the end).

---

## Commands used in these missions

| Command | What it does |
|---|---|
| `/gm s` / `/gm c` | Survival / Creative (Creative lets you fly) |
| `/give <item> [count]` | Gives an item, e.g. `/give site_board`, `/give feast_table`, `/give wheat_seeds 64` |
| `/we pos1`, `/we pos2` | Marks two corners at your feet |
| `/we set <block>` | Fills the marked box, e.g. `/we set wheat_stage_0` or `/we set air` (clears it) |
| `/we size` | Says how big the marked box is |
| `/trial list` / `/trial <id>` / `/trial off` | Lists trials, starts one, leaves one |
| `/time set day` / `/time set night` / `/time set midnight` | Daytime / night-time / the darkest time |
| **F3** | Shows the debug overlay, including **FPS** (frames per second) |

---

## Feedback

At the end, write a short list like this, one line per mission, and hand it to the grown-up who looks after the game. A co-host writes its report in the shape its instructions give instead.

```
Build (bottom of Settings):
Re-checks: torch looks right / torchlight bright at night / small blocks: works / doesn't. What happened:
Mission 0 Torch at midnight: worked / didn't. What happened:
Mission 1 Path: worked / didn't. What happened:
Mission 2 Wheat field: worked / didn't. What happened:
Mission 3 Site Board: worked / didn't. What happened:
Mission 4 Grey Quarry: worked / didn't. Words reached:   Time taken:
Sound, lightning, lag: better / same / worse.
If laggy: where (e.g. mountains), and press F3 and read out FPS, Frame, Tick and Draws:
Best thing:
Anything broken:
```

## Links

- This guide: https://github.com/decented/axenstax/blob/main/docs/missions/latest.md
- Plain text of this guide: https://raw.githubusercontent.com/decented/axenstax/main/docs/missions/latest.md
- Player guide: https://github.com/decented/axenstax/tree/main/docs/player-guide
