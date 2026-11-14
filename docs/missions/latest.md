# Axe'n'Stax test missions: build 0.2.38

This is the test guide for the current build. A tester, or a chat assistant co-hosting their session, reads it for what to type, exact numbers, what success looks like, and why something might not work.

Everything in this file was checked against the game's code for build 0.2.38. If the game and this guide disagree, that is worth reporting.

- **Build:** 0.2.38 (desktop app, Linux)
- **This session:** first the "Ask first" questions, then the "Fixed since last time" re-checks (Mission 0), then the new missions 5 to 8 (off-hand, shield, lantern, tool shapes), then any of the other missions "On your own"
- **Coming next:** missions at a village, and missions with a friend

---

## Ask first: questions from last time

**Co-host:** ask these at the start of the session, before Mission 0, and put the answers in your report under Re-checks.

Last time, the whole inventory disappeared during a fight with brigands. Brigands cannot take items, so we want to find out what happened:

1. When all your things disappeared, did a **"You Died!"** screen come up?
2. Around then, did you type `/gm c`, or switch to Creative some other way?
3. Can you go back to where it happened? Is there a **grave** block there? If there is, right-click it: are your things inside?

---

## Fixed since last time (re-check these first)

These were found in earlier playtests and are fixed by 0.2.38 (some first came out in 0.2.35, 0.2.36 or 0.2.37). Check each one and say whether it now works. Mission 0 below walks through them.

1. **A torch in an item frame looks right.** It lies flat on the frame showing the whole torch, from the front, the side and above, with no black box round it.
2. **A torch dropped on the ground, or held in your hand, is a flat torch, not a dark cube.** The same goes for flowers, saplings, rails and tall grass.
3. **Blighted crops are grey.** A blighted crop is ash grey and half the size of a healthy one, so it is easy to spot next to yellow wheat.
4. **Respawning without a bed puts you on dry land**, near where the world started. Before, it could drop you from the sky into the sea. Worlds made before this build are fixed when they load, unless you set a spawn with a bed or `/sp`.
5. **Satoshi's house is built on dry ground near where you start, and shows up at once.** Before, it could float on the sea and stay invisible until you broke a block nearby. It is never built on top of anything you placed. A house that was already built stays where it is.
6. **Respawning at a bed puts you next to the bed**, inside the room, not on the roof.
7. **Thunder is softer.** It rumbles in just after the flash instead of hitting at full volume, and a bolt far away sounds quieter than one close by. To turn thunder down on its own, press **Esc**, then **Settings**, and move the **Thunder** slider in the sound settings.
8. **You can see what you are holding.** In first person your hand and the item in it now show in the bottom right of the screen, like Minecraft. Before, they sat below the edge of the screen.
9. **Chat starts empty in each world.** Opening a world, or the same world again, no longer shows chat or commands from before.

10. **Tools no longer look like grey slabs in your hand.** In first person, and on other players, every tool has its own shape. A pickaxe has a head across the top of a stick, an axe has a blade on one side, a hoe has a hooked head, and so on. Mission 8 looks at them all.

**New in 0.2.38:**

- **An off-hand.** Press **F** to swap the item in your hand with the item in your off-hand. A box to the left of your hotbar shows it. Mission 5.
- **A shield.** Made at the workbench, held in either hand, raised by holding right-click. It blocks hits from the front. Mission 6.
- **A lantern.** Made at the workbench; stand it on the ground, hang it under a ceiling, or carry it. A lantern or torch in either hand now lights the dark round you. Mission 7.

**New in 0.2.37 (check these if you missed them):**

- **Bigger chat text.** Chat and commands are 1.5 times bigger. To make them bigger still, press **Esc**, then **Settings**, then move the **Chat text size** slider (100% to 300%).
- **The Esc menu button that used to say "Graphics" now says "Settings".**
- **Tab finishes commands, like Minecraft.** Type `/gi` and press **Tab** to get `/give`. Type `/give tor` and press **Tab** to get `/give torch`. Press **Tab** again for the next suggestion, **Shift+Tab** to go back. The suggestions show above the chat line, and you can also click one.
- **Tall grass** in the world now shows only its blades, with no dark green square behind them.
- **A weather command.** `/weather clear` stops rain and keeps the sky clear for 20 minutes. `/weather rain` and `/weather storm` (or `/weather thunder`) start rain or a thunderstorm for 5 minutes. Add a number of seconds to choose how long, up to 3600 (one hour), e.g. `/weather clear 600`. Like other commands, it marks the world as "cheats used". It does nothing in the Workshop or while a trial sets the weather.
- **More games.** On the lobby's **More to play** page, a new **More games** section shows three other games by the same makers: Stampede, Twang Thang and Cluster Flock. They are Android games; **See it** opens their page on the website.

**Known, not fixed yet** (if one of these happens, it is already known):

- A lowered shield is a thin sliver in third person (F5), because you see it edge-on. A raised one is easy to see.
- There is no ranged attack in the game yet, so there is nothing for a shield to block from a distance. It only blocks close hits.
- On another player, their off-hand swing does not show. Their left hand holds still when they use the off-hand.
- A lantern does not fall if the block under it is broken. It stays in the air, like a torch.
- A carried light can glow faintly through a thin wall.

- The world is split into columns 16 by 16 blocks wide. A sealed room built across the edge of one of those columns can stay lit like daytime inside at night, even with no torch.
- A Satoshi house that was already built over the sea stays there.

**Good to know:** brigands cannot take or carry your items. If your whole inventory disappears, you died: your things are in a grave block where it happened.

---

## Changed in 0.2.34 (the last playtest build)

- Torches show their flame and handle, and torchlight at night is about as bright as at noon.
- Buttons, pressure plates and levers show the right picture.
- Rails and cables no longer glow at night, and water next to a torch is lit by it.
- The Workshop is always daytime.

---

## Before every mission (do this once)

1. **Make a new Survival world just for testing.** The missions use chat commands, and commands mark a world as "cheats used". Keep them out of your real world.
2. **Press T** (or /) to open chat and type a command. Press **Tab** to finish a command or an item name.
3. **Type `/gm c`** to switch to Creative so you can fly. Find **flat, open grass with no trees**. Missions 1 to 3 all happen there.
4. **Type `/gm s`** to switch back to Survival. The game must reply **"Switched to survival"** (or "already in survival").
5. **Check the build number** at the bottom of the Settings menu. It should say **v0.2.38**. Use that number in the feedback.

**Good to know:** hunger drops 1 point (half an icon on the hunger bar) every 30 seconds, whatever you are doing. A full bar lasts about 10 minutes, so take some food (berries, bread or cooked meat), or use `/give bread 16` in a test world.

**Why step 4 matters:** worn paths and blight (missions 1 and 2) never happen in Creative. Creative also stops places from looking grey. If you can still fly, you are in Creative and those missions will not work.

---

## Mission 0: Re-checks for 0.2.38

**What it tests:** the fixes in "Fixed since last time".

1. **Item frame:** type `/give item_frame` and `/give torch 4`. Place the frame on a wall, then right-click the frame with the torch in your hand to put it in. Look at it from the front, from the side and from above.
2. **Dropped and held torch:** press **Q** with a torch selected to drop one, and look at it on the ground. Look at the torch in your hand in the bottom right of the screen, then press **F5** to see yourself holding it (press F5 again to go back).
3. **Blight:** if you still have a giant wheat field from last time, look for grey plants. Otherwise do Mission 2 and check the colour.
4. **Respawn:** in your test world, with no bed, type `/kill` or let something defeat you, then respawn. Say where you came back.
5. **Chat:** type `/gi` and press **Tab**. Then try `/give tor` and **Tab**, and **Tab** again. Then change **Esc, Settings, Chat text size** and see the chat get bigger.
6. **Thunder and weather:** type `/weather storm` and wait for lightning (a bolt comes about every 15 seconds in a storm, sometimes sooner or later). Say whether the thunder still makes you jump. Then type `/weather clear` and check the rain stops.
7. **Chat:** type a few lines, leave to the lobby, open the same world again, and check the chat is empty.
8. **Tool shapes:** type `/give iron_sword` and `/give iron_pickaxe`, hold each, and look at your hand. Neither should be a grey slab. (Mission 8 looks at all twelve.)

**Success:** the framed torch is a flat torch from every side with no black box; the dropped and held torch are flat torches; blighted wheat is grey; you respawn standing on dry land; Tab finishes the words and the chat text size changes; the thunder rumbles in after the flash and is not a jump scare; `/weather clear` stops the storm; you see the torch in your hand; the chat is empty after reopening the world; the sword and pickaxe in your hand are different shapes, not slabs.

**If it does not work:**
- Anything still looks like the old build: check the build number at the bottom of Settings says **v0.2.38**.
- Tab does nothing: Tab only completes lines that start with `/`.

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

## Mission 5: The off-hand

**What it tests:** the second hand. **F** swaps, and right-click uses the off-hand when the main hand has nothing to do.

**Type:** `/give iron_pickaxe`, `/give torch 16`, `/give iron_sword`, `/give bread 16`

**Do:**
1. Put the pickaxe in hotbar slot 1 and the torch in slot 2. Select slot 2 and press **F**. The torch moves to the box on the **left of the hotbar**, and slot 2 is now empty.
2. Select slot 1 (the pickaxe). Point at the ground or a wall and **right-click**. A torch goes down, and the pickaxe stays in your hand.
3. **Mine** a block with **left-click**. It uses the pickaxe, never the torch.
4. Open the inventory with **E**. The off-hand slot is next to the boots. Click it to pick the torches up and put them back.
5. Put the sword in slot 1, put the bread in the off-hand (select it, press **F**, then select the sword). Wait until the hunger bar is down by at least 1 point (hunger drops 1 point every 30 seconds), then **right-click** with the sword selected. You eat the bread from the off-hand.
6. Press **F5** and look at yourself: the torch (or the bread) is in the **left** hand.

**Look for:**
- **F** swaps the two items every time.
- A torch in the off-hand lights the dark round you, same as a torch in the main hand.
- With a **sword** or **pickaxe** selected, right-click uses the off-hand item. With a **torch** or **bread** selected, it uses the selected one first.
- Left-click never uses the off-hand.

**The rules behind it:**
- The off-hand holds one stack of any item.
- Right-click uses the main hand first. Only if the main-hand item has nothing to do with what you point at does the off-hand item get the click. A hoe on stone has nothing to do, so a torch in the off-hand goes down; a hoe on grass still tills.
- A bow or slingshot takes its ammo from the off-hand first.
- If you are defeated, the off-hand item stays with you. It does not go in the grave.
- The off-hand is saved with the world.

**If it does not work:**
- **F** does nothing while you are painting a skin in the Workshop (the paint tool uses that key).
- Eating needs you to be hungry or hurt.

---

## Mission 6: The shield

**What it tests:** making a shield, raising it, blocking a hit, and its wear.

**Do:**
1. **Make one.** At a workbench (3 by 3 grid) put planks, iron ingot, planks along the top; planks, planks, planks across the middle; one plank at the bottom middle. That is a Y shape. (Or `/give shield`, but make one at least once.) Test items: `/give planks 6`, `/give iron_ingot`.
2. Hold the shield in either hand. Point at **open air or plain ground** and **hold right-click**. Count a quarter of a second (a quick "one-two"); the shield lifts in front of you.
3. Point at a **chest or a door** and right-click: that opens or toggles it, and the shield does **not** raise. Do the same at a **sheep or cow** with wheat or a Lead in your other hand: it takes the click (feeding, or the Lead) and the shield stays down.
4. With the shield up, **walk**, then try to sprint. Then **left-click** a block and a mob.
5. Type `/gm s`, then `/spawn brigand`. Raise the shield while the brigand walks at you from the front: no damage, a clack. (The game has no zombies; the brigand is the enemy to use.) Keep it up while the brigand hits you, and look at the shield's durability bar after each clack. Once the shield is up, it stays up while you hold the button, even with the brigand under the crosshair.
6. Turn so the brigand is **behind** you, with the shield still up.

**Look for:**
- The shield is a wooden board with a grey rim and a round grey middle.
- A hit from **in front** does no damage and no push, and you hear a **clack**.
- A hit from **behind** hurts as normal.
- You walk **slowly** (like sneaking) and cannot sprint.
- Left-click does nothing, and a block you were breaking stops cracking.

**The rules behind it:**
- A new shield has **336** durability.
- A blocked hit costs the shield **1 plus the damage it stopped** (rounded down). A brigand hit of 3 on Normal costs 4. One brigand in front wears it about once every half second, with one clack.
- The shield starts blocking after **0.25 seconds** (5 ticks) of right-click held. Letting go lowers it straight away.
- It blocks hits from the front half: within 90 degrees of where you look, flat across the ground. Height does not matter.
- It does not block lava, fire, falling, drowning or hunger.
- Hold right-click to raise it. A chest, a door, or a mob you can actually do something to (feeding, taming, a Lead, milking, shearing, riding, trading, or a command to your own pet) takes the click instead, and the shield stays down there.
- An enemy you cannot use anything on (a brigand, marauder or berserker) does not take the click, so facing one the shield goes up.
- Once raised, the shield stays up while you hold the button, whatever comes under the crosshair.
- With a shield in the main hand and a torch in the off-hand, right-clicking stone places the torch.
- In a multiplayer game the host's game decides what the shield blocks.
- A shield that is used up breaks and disappears.

**If it does not work:**
- Make sure you are not pointing at a chest, a door, or an animal or villager you can use something on.
- Hold the click for at least a quarter of a second before the hit.
- Check you are in Survival (`/gm s`): in Creative, nothing hurts you.

---

## Mission 7: The lantern

**What it tests:** the lantern, and light carried in your hand.

**Do:**
1. **Make one** at a workbench: an iron ingot, a torch, an iron ingot in a column (top, middle, bottom). Makes 1 lantern. (Or `/give lantern`.) Test items: `/give iron_ingot 2`, `/give torch`.
2. **Stand it:** right-click the **top** of a block.
3. **Hang it:** right-click the **underside** of a block (a ceiling).
4. Pick it up by breaking it, and check you get the lantern back.
5. **Carry it:** type `/time set midnight` and go into a closed room or cave with no torches. Hold the lantern in your main hand and look around. Press **F** to move it to the off-hand and look again. Then try a torch in either hand.

**Look for:**
- A lantern is a dark iron frame with a warm yellow middle. It looks different standing and hanging (a hanging one has a little chain).
- A **lantern or a torch in either hand** lights the room round you, brightest next to you and fading with distance.
- Another player holding one lights the room round them too.

**The rules behind it:**
- A lantern gives light level **15**. A torch gives 14 and a paper lantern 12.
- It can be held, stood or hung. It is one item.
- Held lights light the area for you and other players, nearest first, up to 8 lights at once.
- Carried light is for looking only. It does not make mobs stop spawning and does not help crops grow.
- A lantern stays in place if the block it sits on is broken.

**If it does not work:**
- If the room is not dark, the carried light is hard to see. Use `/time set midnight` and a sealed room.

---

## Mission 8: Tool shapes

**What it tests:** every tool has its own shape in your hand.

**Type:** `/give iron_pickaxe`, `/give iron_axe`, `/give iron_sword`, `/give iron_shovel`, `/give iron_hoe`, `/give bow`, `/give fishing_rod`, `/give shears`, `/give shield`

**Do:** hold each tool in turn and look at your hand in the bottom right. Then press **F5** to see it in your hand from outside.

**Look for:**
- The **sword** is **long and pointed**, with a crossguard wider than the blade.
- The **fishing rod** shows a **thin line hanging from the tip with a small red float** on the end.
- The pickaxe, axe, shovel, hoe, bow and shears are each different, not the same slab.
- Another player holding a tool shows the same shape in their hand.

**The rules behind it:**
- Every tool type has its own shape, in first person and in third person.
- A tool made of a different material is the same shape in a different colour.
- The axe and the shovel can look alike at a glance (known; the shovel is flat and centred).

**If it does not work:** tell us which tool still looks like a slab. A tool from a newer or older build of the game may show as a plain slab.

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
| `/give <item> [count]` | Gives an item, e.g. `/give site_board`, `/give feast_table`, `/give wheat_seeds 64`, `/give shield`, `/give lantern` |
| `/spawn <mob>` | Puts a mob 2 blocks east of you, e.g. `/spawn brigand` |
| `/we pos1`, `/we pos2` | Marks two corners at your feet |
| `/we set <block>` | Fills the marked box, e.g. `/we set wheat_stage_0` or `/we set air` (clears it) |
| `/we size` | Says how big the marked box is |
| `/trial list` / `/trial <id>` / `/trial off` | Lists trials, starts one, leaves one |
| `/time set day` / `/time set night` / `/time set midnight` | Daytime / night-time / the darkest time |
| `/sp` | Sets where you respawn to where you stand |
| `/kill` | Defeats you straight away (to test respawning) |
| `/weather clear` / `/weather rain` / `/weather storm` `[seconds]` | Clear sky (20 minutes) / rain / thunderstorm (5 minutes), or for that many seconds |
| **Tab** / **Shift+Tab** | Finishes a command or item name / goes back one suggestion |
| **F** | Swap what is in your hand with your off-hand |
| **F5** | Camera behind you (see what you are holding) |
| **F3** | Shows the debug overlay, including **FPS** (frames per second) |

---

## Feedback

At the end, write a short list like this, one line per mission, and hand it to the grown-up who looks after the game. A co-host writes its report in the shape its instructions give instead.

```
Build (bottom of Settings):
Ask first (brigand fight): "You Died!" screen? / typed /gm c? / grave there, things inside?:
Re-checks: framed torch / dropped and held torch / grey blight / respawn on dry land / bed respawn / chat Tab and text size / softer thunder / /weather: works / doesn't. What happened:
Mission 0 Re-checks: worked / didn't. What happened:
Mission 1 Path: worked / didn't. What happened:
Mission 2 Wheat field: worked / didn't. What happened:
Mission 3 Site Board: worked / didn't. What happened:
Mission 4 Grey Quarry: worked / didn't. Words reached:   Time taken:
Mission 5 Off-hand (F swap / torch while mining / food / left hand in F5): worked / didn't. What happened:
Mission 6 Shield (recipe / raise after a quarter second / clack from the front / hit from behind / slow walking / no mining while up / durability): worked / didn't. What happened:
Mission 7 Lantern (recipe / stand / hang / carried light in either hand): worked / didn't. What happened:
Mission 8 Tool shapes (sword long and pointed / rod line and float / other tools different): worked / didn't. What happened:
Sound, lightning, lag: better / same / worse.
If laggy: where (e.g. mountains), and press F3 and read out FPS, Frame, Tick and Draws:
Best thing:
Anything broken:
```

## Links

- This guide: https://github.com/decented/axenstax/blob/main/docs/missions/latest.md
- Plain text of this guide: https://raw.githubusercontent.com/decented/axenstax/main/docs/missions/latest.md
- Player guide: https://github.com/decented/axenstax/tree/main/docs/player-guide
