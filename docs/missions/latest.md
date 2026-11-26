# Axe'n'Stax test missions: build 0.2.42

This is the test guide for the current build. A tester, or a chat assistant co-hosting their session, reads it for what to type, exact numbers, what success looks like, and why something might not work.

Everything in this file was checked against the game's code for build 0.2.42, and the new looks (the torch in the fist, icons, armour, the fishing line, eating) were checked in screenshots. If the game and this guide disagree, that is worth reporting. The look of Cobbled Deepslate was not checked by eye, and your opinion on it is wanted.

- **Build:** 0.2.42 (desktop app, Linux). There is no phone build of 0.2.42: the phone app stays at 0.2.41, so phone testing waits for the next build.
- **Playing with a friend:** 0.2.42 changes the network protocol (version 101 becomes 102). A 0.2.41 player cannot join a 0.2.42 world, and a 0.2.42 player cannot join a 0.2.41 world: the join is refused. Both players must be on 0.2.42. That includes the phone app, which stays on 0.2.41 for now, so a phone cannot join a desktop 0.2.42 world.
- **This session:** first the "Ask first" question, then Mission 0 (the re-checks: everything new in 0.2.42, then the older ones not tried last time), then missions 6 to 8 (shield, lantern, tool shapes), then any of the other missions "On your own"
- **Coming next:** missions at a village, and missions with a friend

---

## Ask first: from last time

**Co-host:** ask these at the start of the session, before Mission 0, and put the answers in your report under Re-checks.

1. **Do you ever play with a game controller** (an Xbox or PlayStation pad), or always with the keyboard and mouse? Just the answer is enough; nothing to test. It was not answered last time. It tells us whether a controller-only bug needs fixing soon.
2. **If your things ever disappear again, stop and tell us straight away**, before doing anything else. (Nothing disappeared last session, but we still don't know what emptied it before.) Could you fly (Creative) at that moment? Was any message or red screen on screen? What was near you?

---

## New in 0.2.42 (re-check these first)

These answer the last playtest. Mission 0 below walks through them in an order that saves typing. Each one gives the steps and what a pass looks like.

### 1. Settings, the pause menu and the relays window scroll

- **Steps:** press **Esc**, then **Settings**. Roll the **mouse wheel** over the list. If you can, make the game window a little shorter (about 700 pixels tall) and look again. Then press **Esc** and look at the pause menu the same way.
- **Pass:** at any window size you can scroll Settings down to its last line (**AxeNStax v0.2.42**), and the **Back** button stays on screen under the list. The pause menu scrolls too: in Survival you can reach **Switch to Creative**, its last button. Before, a normal desktop window showed the whole panel with no scrolling, and the bottom ran off the screen.
- The relays window (the **Relays** button on the sign-in dialog, also in the lobby) scrolls the same way when its list is long; its **Done** button is at the bottom of the scrolled list.

### 2. The grave is visible

- **Steps:** in Survival, put a few items in your bag (`/give dirt 4`), type `/kill`, and press **Respawn**. Walk back to where you died. Then right-click the grave.
- **Pass:** a grave (a weathered stone slab on an earth mound) stands where you died, drawn at once. Before, only its outline showed when you looked at it. After you right-click it and get your things back, the grave **disappears at once**, with no ghost block and no outline left.

### 3. `/give` works for every block and item, by its name

- **Steps:** type `/give spruce planks`, `/give deepslate`, `/give cobbled deepslate`, `/give granite`, `/give obsidian`, `/give birch log`. Try the name as you see it in the game, or the registry name (`/give genesis:spruce_planks`). Press **Tab** after a few letters to see suggestions. If your bag can only hold part of what you asked for, it says how many fitted ("gave N of M"). A name that ends in a number, such as `/give wheat stage 2`, is tried as a whole name first.
- **Pass:** each gives that item. `/give deepslate` gives **Pure Deepslate** (the block as it is in the ground); `/give cobbled deepslate` gives **Cobbled Deepslate** (what mining deepslate gives; see 12).
- **Five things can't be given**, because they are marks the world makes and not items: air, the piston's head, a Feast Marker, a Worn Path and a Blighted Crop. `/give` says it doesn't know them. Everything else the game has can be given. A few old names still give their old thing: `/give copper` gives the ore (the raw metal is `raw_copper`), `/give iron chest` is the chestplate (the chest block is `iron_chest_block`), `/give mill` is the windmill (`grain_mill` is the mill).

### 4. The furnace: a new screen

- **Set up:** `/give furnace`, `/give raw_iron 8`, `/give spruce planks 8`, `/give birch planks 8`, `/give coal 2`, `/give raw_copper 2`. Place the furnace and right-click it.
- **The screen:** **Furnace** at the top, a **Move whole stacks** tick box, then **Input**, **Fuel** and **Output**, two bars (smelting and burning), and under them **Your inventory**: your bag in three rows and your hotbar under them. Every slot shows the item's **icon and count**, as the inventory does, and all four inventory rows fit on the screen (in a very short window the panel scrolls). Things the furnace will smelt (raw iron, raw copper, tin) have a **blue edge**. Things it will burn (coal, planks, logs, sticks, leaves) have an **orange edge**. Under Input, Fuel and Output the item's plain name is written.
- **Steps:**
  1. Click the **raw iron**. One goes into **Input** by itself. Click the **spruce planks**. One goes into **Fuel**.
  2. The smelting bar fills for **10 seconds** and the fuel bar says **burning: about 12 s left**. Then **Output** shows **Iron Ingot**. Click **Output** to take it.
  3. Hold **Shift** while you click a stack (or tick **Move whole stacks**): the whole stack goes in, not one.
  4. Click **Input** or **Fuel** to take what is in it back to your bag.
  5. Try a wrong move: click an item with no blue or orange edge (dirt). With raw iron already in Input, click the raw copper (it is named **Copper** in the inventory).
  6. Press **E** or **Esc** to close.
- **Pass:** every click either moves something or says why in orange text for about 3 seconds ("That can't be smelted or burned", "The Input slot holds something else: take it out first", "The Input slot is full", "That won't burn"). Names are plain ("Raw Iron", "Oak Planks"), never code names. If your bag has no room at all, taking something out of Input, Fuel or Output says **"Your inventory is full"** and takes nothing. Before, the screen could only load the item selected on the hotbar and said nothing when it refused.
- **The plank-fuel re-check from 0.2.41 (planks of every tree burn):** for each of oak, birch, spruce, jungle, acacia and dark oak planks, put one raw iron in Input and one plank of that tree in Fuel. **Pass:** every one smelts an Iron Ingot. A plank burns 12 seconds and one smelt takes 10, so one plank is enough for one raw iron.

### 5. The shield stops a brigand without pushing you back

- **Steps:** `/gm s`, `/give shield`, then `/spawn brigand` on open ground with nothing behind you. Hold **right-click** to raise the shield, facing the brigand. Stay still for about 10 seconds. Then turn so it is behind you, shield still up.
- **Pass:** the brigand walks up and stops against you. You are **not pushed back**, take **no damage**, and hear a **clack** on each blocked hit (the shield's durability bar drops). From behind, the hits hurt as normal. Before, the brigand kept walking into you and slid you backwards even though the hits were blocked.
- **Known, small:** if the brigand is standing one block **higher** (on a ledge) and steps down to you, you can be nudged about 1.5 to 2.5 blocks once, as it drops. After it lands it holds still. That is not fixed.

### 6. A held torch sits in the fist in third person

- **Steps:** `/give torch 4`, hold a torch, press **F5**. Stand still, walk, jump, and left-click to swing. Press **F** to move it to the other hand and do it again. F5 cycles three views (first person, over the shoulder, behind you). To see yourself from the front, first tick **Esc, Settings, Free-look (hold Alt to look around)**, then hold **Alt** and move the mouse while in F5.
- **Pass:** the torch is gripped in the fist, standing upright above it with its foot just under the hand, in **either** hand, in every one of those poses. The forearm lifts forward a little so the flame is clear of your arm. Before, it floated beside you. The same grip holds for any flat item (a sapling, bread).

### 7. Icons look like the thing

- **Steps:** `/give iron_pickaxe`, `/give iron_sword`, `/give satori helmet`, `/give diamond chestplate`, `/give spruce fence gate`, `/give spruce planks`, `/give spruce slab`, `/give spruce stairs`, `/give spruce door`, `/give spruce trapdoor`, `/give granite wall`, `/give spruce fence post`, `/give birch fence post`, `/give plan`, `/give iron_ingot`, `/give lantern`, `/give water_bucket`, `/give lava_bucket`, `/give copper_ingot`, `/give tin_ingot`, `/give bronze_ingot`. Open the inventory (**E**) and look at the hotbar too.
- **Pass:** tools show the tool's own shape (a pickaxe has its head across the top of a stick), coloured by material. Armour shows a silhouette of the piece (helmet, chestplate, leggings, boots) in its material colour. A **Plan** shows a rolled blueprint. A **slab** is the lower half of the square, **stairs** are the lower half plus an upper back step, a **door** is one tall narrow panel with a handle, a **trapdoor** is a thin lid with two hinges, a **fence gate** is two posts with two rails, a **wall** is a tall post with a low wall each side, a **fence post** is a narrow post with rail stubs. The **spruce fence gate** now looks different from **spruce planks**, and the spruce and birch fence posts each show their own wood. The water bucket is blue and the lava bucket orange, and the copper, tin and bronze ingots each have their own colour (for a time they were all drawn the same). No icon is a flat coloured square with a letter.

### 8. Armour shows: worn, held and dropped

- **Steps:** `/give satori armour` (four pieces). Press **E**, pick up each piece and click the matching slot in the inventory's armour column (helmet at the top down to boots, left of the crafting grid). Press **F5**. Walk, jump and swing. Then put one piece in your hotbar, select it, and look at your hand in first person and in F5. Then press **Q** to drop it and look at it on the ground.
- **Pass:** in F5 all four worn pieces show on your character, the helmet on the head, the chestplate on the body and arms, the leggings on the legs and the boots on the feet, and they swing with the limbs. A piece in the hand shows as a small model of that piece (not an empty hand). A dropped piece is a small model on the ground. You cannot pick up your own **Q**-drop for 1.5 seconds. After that it is pulled back to you if you are within 2.4 blocks, so look at it at once, or step back more than 3 blocks. Before, none of these showed.
- **Armour numbers** (the number beside the hearts, with a shield sign; it is hidden when you wear nothing). Points per piece, helmet / chestplate / leggings / boots: **Leather** 1 / 3 / 2 / 1 = 7. **Chainmail** 2 / 5 / 4 / 1 = 12. **Iron** 2 / 6 / 5 / 2 = 15. **Diamond** 3 / 8 / 6 / 3 = 20. **Satori** 4 / 9 / 7 / 4 = 24. **Rubber** exists only as boots (2). So a full Satori set reads **24**, take the chestplate off and it reads **15**, put it on and it is **24** again. That is correct.
- **Known gap:** other players do not see your worn armour yet. They do see a piece you hold in your hand (both of you on 0.2.42).

### 9. Fishing: a bobber on a line

- **Steps:** `/give fishing_rod`. Stand within 12 blocks of water and look at it. **Right-click**. Wait 5 to 15 seconds. When **"A bite! Right-click to reel it in."** shows, right-click again. Do it again, and this time reel in before any bite. Press **F5** and cast again.
- **Pass:** on the cast, a small red-and-white **bobber** flies from the rod tip to the water on an arc and floats there, bobbing, on a thin **line** that sags from the rod tip. The text says **"Line cast. Wait for a bite…"**. While the line is out, the rod in your hand has **no float or hook of its own**. On the bite the bobber is pulled under the water with a splash and jitters until you reel it in, and you get **"You caught N× ..."**. Reeling early says **"You reeled in early — nothing bit."** The rod's own line and float come back when it is reeled in. The bobber floats on the real water surface (lower on shallow, flowing water). The line is drawn only from the rod and hand you cast with: select another hotbar slot (even one holding a second rod) and the line is no longer drawn from the new rod. Dying also drops the line. Looking away from water says **"Aim at water to fish."** Before, the hook and float stayed on the rod and nothing was drawn on the water.
- **Known gap:** other players do not see your line or bobber yet. Their view of your rod shows the old line and float on it.

### 10. Eating you can see

- **Steps:** `/give bread 8`. Wait until the hunger bar has dropped by at least 1 point (hunger drops 1 point every 30 seconds). Hold the bread and **right-click**. Try two other foods (for example `/give carrot` and cooked meat). Press **F5** and eat again.
- **Pass:** your hand brings the food up to your mouth. The food loses **three bites**, each a notch cut out of its picture, and **three** small bursts of crumbs in the food's colours fall from the mouth, each with a munch sound. One item takes **0.8 seconds**. Holding right-click eats the next item of the stack every 0.8 seconds. Moving to another hotbar slot or opening a screen stops the picture. In F5 your arm swings in and the food is held **in front of your face**, with the crumbs coming from there. The picture also stops if you swap hands (**F**) or pause, and, when you play on a friend's world, if the host refuses the eat (nothing is eaten then). Eating works the same as before; only the picture is new.
- **Known gap:** other players do not see you eat yet.

### 11. Torches and lanterns drop when what holds them goes

- **Steps:** `/give torch 4`, `/give lantern 2`, `/give dirt 8`. In Survival, build on open flat ground (dirt breaks in 2.5 seconds by hand). Put a torch on top of a single dirt block, and break that block. Put another torch on the ground **right beside a wall of dirt**, then break the ground it stands on (the torch must drop even though the wall is still there). Then build a pillar 3 blocks high on its own, put a torch on the **side** of the top block (nothing else touching the torch), and break the top block. Stand a lantern on a block, and break the block. Hang a lantern under a ceiling block, and break the ceiling block. If you can, hang a second lantern on the underside of the first, and break the ceiling.
- **Pass:** each torch and lantern turns into an item on the ground within about half a second, and you can pick it up. A lantern hung under another lantern falls a moment after the first. Before, they stayed floating in the air.
- **Rules to know:** a **torch** remembers the block it was **placed against** (floor, wall or ceiling) and drops when that block goes, even if another block still touches it. A torch placed before 0.2.42, or against something that is not solid, keeps the old rule: it stays while any solid block touches it. A **standing lantern** needs the block **below** it. A **hanging lantern** needs a solid block or another lantern **above** it. A lantern you click onto the **side** of a wall counts as standing, so with nothing under it it drops at once. Blasts, pistons, falling blocks and fire all count too, not only your hand. A torch or lantern **you placed** that was already floating in an old world drops the first time that part of the world loads in 0.2.42 (torches and lanterns that came with the world, such as in villages, are left alone). If you quit while a lantern chain is still falling, the links still hanging are saved as they are and fall when you load the world again, each dropping one lantern.

### 12. Cobbled Deepslate

- **Steps:** `/gm s`, then `/give stone_pickaxe`. In a cave or deep underground, find **deepslate** (dark grey stone) and break it (or `/give deepslate 4`, place it and break it: placed deepslate drops the same). Then `/give cobbled deepslate 12`, `/give stick 4` and make a **stone pickaxe** at a workbench (3 by 3 grid): three Cobbled Deepslate across the top, a stick in the middle and a stick below it. Then three Cobbled Deepslate in a row (6 deepslate slabs), then 2 rows of 3 (6 deepslate walls).
- **Pass:** breaking deepslate drops **Cobbled Deepslate**, not deepslate. Pure deepslate takes 15 seconds to break at hand speed and gives a drop only to a **stone pickaxe or better** (a wooden pickaxe breaks it and drops nothing). Cobbled Deepslate takes 3.5 seconds at hand speed (cobblestone takes 2) and needs a pickaxe to drop. It counts as cobblestone wherever cobblestone is the stone ingredient: stone pickaxe, axe, shovel, hoe and sword, a stone bow, a furnace (ring of 8), a lever, a dispenser, a dropper, a piston, a salt lick and a Commons Stone, with the same shapes as cobblestone. A mix of cobblestone and cobbled deepslate also works. Deepslate slabs, stairs and walls craft from it, and a cobbled deepslate recipe never makes a cobblestone slab, stairs or wall. The recipe book cards for deepslate slabs, stairs and walls, and **Fill from bag**, accept it too.
- **Looks:** it is dark cobblestone, the cobblestone picture darkened (no new art), so it looks like a darker cobblestone. From far away, and in the Workshop's painter, it still looks like plain cobblestone. The recipe book card text still says "cobblestone". Not checked by eye.
- A Satori vein still drops its gem alongside, as before.

### 13. Creative bedrock

- **Steps:** `/gm c`, `/give bedrock`, place a bedrock block and **left-click** it. Then fly down to the bottom of the world (the lowest layer, y = 0) and try to break that. Then `/gm s`, place bedrock again and try to break it.
- **Pass:** bedrock **you placed** breaks in **Creative**. Bedrock the **world made** (the bottom layer) never breaks, in any mode. Bedrock you placed does **not** break in Survival. Bedrock made with `/we set bedrock` counts as placed (so it breaks in Creative), and `/we set air` clears that mark. Explosions never break bedrock. The "you placed it" mark is saved with the world.
- **In a friend's world** (both on 0.2.42): in Creative your break is sent to the host, who decides. Bedrock placed by a player breaks. If you try to break the world's own bedrock, the block may flicker away for a moment and then come back: that is the host refusing it.

---

## Answers to last time's findings

- **The Site Board brightening** was over-promised. The confetti is the signal; the change to the place is slow. See Mission 3, now corrected.
- **Storms were not always 5 minutes by nature.** `/weather storm` with no number lasts **300 seconds (5 minutes)**, because that is its default. Natural storms are shorter and do vary: **60 or 120 seconds** (two in three are 60). `/weather storm 90` sets your own length (seconds, up to 3600).
- **The fishing rod breaks blocks like a bare hand, on purpose.** Any item that is not the right tool mines at hand speed. Nothing is built to stop it.
- **A blocked brigand hit "knocked you back":** it was not knockback. The brigand walked into you and slid you along. Fixed, see 5.
- **Armour went 24, then 15, then 24:** correct, see the numbers in 8.
- **Not built:** free typing on Site Board jobs was declined (picked jobs only, for child safety). A spyglass in place of the zoom key (**C**) is parked for later.

**Fixed since last time** (the co-host: re-check each, one line of pass or fail):

1. Settings, the pause menu and the relays window scroll (New 1).
2. Inventory icons for tools, armour, the Plan, gates, slabs, stairs, doors, trapdoors, walls and fence posts (New 7).
3. A piece of armour in your hand shows (New 8).
4. Worn armour shows on your character in F5 (New 8).
5. `/give spruce planks` and `/give deepslate` work, and every block and item has a name (New 3).
6. The spruce fence gate icon is no longer the plank icon (New 7).
7. The grave is visible and vanishes when reclaimed (New 2).
8. Deepslate drops Cobbled Deepslate, which makes stone tools (New 12).
9. The furnace screen shows your whole inventory and says why it refuses (New 4).
10. A cast fishing rod throws a bobber on a line (New 9).
11. A held torch sits in the fist in F5, in either hand (New 6).
12. Torches and lanterns drop when their block goes (New 11).
13. A raised shield facing a brigand: no pushing back (New 5).
14. Bedrock you place can be broken in Creative (New 13).
15. Eating shows (New 10).
16. The furnace screen uses item icons, fits all four inventory rows, and says "Your inventory is full" (New 4).
17. Bucket, copper, tin and bronze icons keep their colours (New 7).

**Confirmed passed last time** (no need to repeat): logs keep their tree; a birch pickaxe; the lantern icon and its hold; the off-hand in the grave; `/give` armour; changing keys; directional jumping legs; Creative blocked while dead; softer thunder; worn paths (7 of 9 after 12 round trips); Site Board saving, ticking and removing; the shield with an off-hand, gates, either hand, no sprint or mining; hits from behind hurting; lanterns standing, hanging, recovered and lighting; a torch lighting from either hand.

**Known, not fixed yet** (if one of these happens, it is already known):

- Other players do not see your worn armour, your fishing line or bobber, or you eating.
- A brigand stepping down off a one-block ledge can nudge a shielded player 1.5 to 2.5 blocks once.
- A lowered shield is a thin sliver in third person (F5), because you see it edge-on. A raised one is easy to see.
- There is no ranged attack in the game yet, so there is nothing for a shield to block from a distance. It only blocks close hits.
- On another player, their off-hand swing does not show. Their left hand holds still when they use the off-hand.
- A carried light can glow faintly through a thin wall.
- `/kill` does nothing in Creative (nothing can hurt you there), even though it says "You died.".
- Playing with a friend: when the friend gets their things back from a grave, their off-hand item goes into their bag, not their off-hand.
- A lantern in an item frame still shows as a small yellow cube.
- A few trial and loading-screen tips still name the default keys ("press E", "press 1") even after you change them.
- The world is split into columns 16 by 16 blocks wide. A sealed room built across the edge of one of those columns can stay lit like daytime inside at night, even with no torch.
- A Satoshi house that was already built over the sea stays there.
- Cobbled Deepslate looks like plain cobblestone far away and in the Workshop, and the recipe book card text still says "cobblestone".
- Floating torches and lanterns already in an old world stay until something next to them changes.
- 0.2.41 and 0.2.42 players cannot play together (protocol 101 against 102).

**From earlier builds, for reference** (already shipped; use these to rule on a tester's remark):

- **Wood (0.2.41):** every tree's log keeps its type (a birch log is birch and makes birch planks). Every log takes about 4.5 seconds by hand, planks 2 seconds, leaves a fraction of a second. Planks of every tree burn 12 seconds. Wherever the kind of wood doesn't matter (crafting table, sticks, wooden tools, a chest), any wood works, even mixed. Something made of one wood, like a fence post or a door, is made of the wood you used. **Fill from bag** in the recipe book follows the same rule.
- **Lantern (0.2.41):** drawn as a lantern with a handle in the inventory and hotbar; a small lantern when dropped (**Q**); held by its handle in first person and in F5.
- **Off-hand at death (0.2.41):** the off-hand item goes into the grave with the rest, armour stays on you, and reclaiming puts it back in the off-hand (or the bag if the off-hand is full).
- **`/give` armour (0.2.41):** `/give satori armour` gives the four Satori pieces, `/give satori helmet 2` gives two helmets. "satoshi" works for "satori", "armor" for "armour".
- **Deleting a world (0.2.41):** **Delete** opens "Delete this world?" with **Cancel** (selected) and **Delete**. Enter cancels; only a click deletes.
- **Controls (0.2.41):** **Esc, Settings, Controls**: each action has a **Change** button. A key already used swaps with it and a line says so. **Reset all to defaults** asks first. **Esc** and **F1 to F12** can't be changed.
- **Shield and a crafting table (0.2.41):** a crafting table works like a chest. Right-click it with the shield down and it opens. If you are already holding right-click with the shield raised, the shield stays up whatever you point at, so nothing opens. That is on purpose.
- **Creative while dead (0.2.39):** `/gm c` on the "You Died!" screen says "Press Respawn first." and you stay dead.
- **Jumping legs (0.2.39):** jumping forwards swings the legs back, backwards swings them forwards, on the spot keeps them straight.
- **Respawn and house (earlier build):** respawning with no bed puts you on dry land near where the world started; Satoshi's house is built on dry ground near the start and shows at once.
- **Off-hand, shield, lantern (0.2.38):** **F** swaps the main hand and off-hand (the box left of the hotbar). A shield made at the workbench blocks hits from the front. A lantern or torch in either hand lights the dark round you. Missions 5 to 7.
- **Chat and commands (0.2.37):** chat starts empty in each world. Chat text is 1.5 times bigger, and **Esc, Settings, Chat text size** goes from 100% to 300%. The Esc menu button is **Settings**. **Tab** finishes commands and item names. Tall grass shows only its blades. `/weather` sets the sky (see Commands). The lobby's **More to play** page has a **More games** section with three other games by the same makers (Android; **See it** opens their page).
- **Thunder (earlier build):** it rumbles in just after the flash, and a far bolt is quieter than a close one.

**Good to know:** brigands cannot take or carry your items. If your whole inventory disappears after a death, your things are in a grave block where it happened.

---

## Before every mission (do this once)

1. **Make a new Survival world just for testing.** The missions use chat commands, and commands mark a world as "cheats used". Keep them out of your real world.
2. **Press T** (or /) to open chat and type a command. Press **Tab** to finish a command or an item name.
3. **Type `/gm c`** to switch to Creative so you can fly. Find **flat, open grass with no trees**. Missions 1 to 3 all happen there.
4. **Type `/gm s`** to switch back to Survival. The game must reply **"Switched to survival"** (or "already in survival").
5. **Check the build number** at the bottom of the Settings menu (**Esc**, **Settings**, scroll to the end). It should say **AxeNStax v0.2.42**. Use that number in the feedback.

**Good to know:** hunger drops 1 point (half an icon on the hunger bar) every 30 seconds, whatever you are doing. A full bar lasts about 10 minutes, so take some food (berries, bread or cooked meat), or use `/give bread 16` in a test world.

**Why step 4 matters:** worn paths and blight (missions 1 and 2) never happen in Creative. Creative also stops places from looking grey. If you can still fly, you are in Creative and those missions will not work.

---

## Mission 0: Re-checks for 0.2.42

**What it tests:** everything in "New in 0.2.42", then the older re-checks that were not tried last time.

**0.2.42 first.** Do items 1 to 13 of "New in 0.2.42", in this order so one set of items serves several: 1 (Settings), 3 (`/give` names), 7 (icons), 8 (armour: wear it, then F5 and the numbers), 6 (torch in F5), 4 (furnace, with the planks of each tree), 12 (cobbled deepslate), 11 (torches and lanterns dropping), 13 (bedrock), 2 (the grave: it uses `/kill`, so do it when your bag holds only test items), 5 (shield and brigand), 9 (fishing, near water), 10 (eating).

**Success:** each one passes as written in its own block. Say pass or fail for each number, and quote any text the game showed.

**Then the 0.2.41 re-checks that were not tried last time:**

1. **Equal log break speeds:** in Survival, with no tools, break one birch log and one oak log by hand. Count roughly how long each takes. Planks of any tree take about 2 seconds by hand and leaves a fraction of a second. **Pass:** both logs take about **4.5 seconds**.
2. **Delete a world:** make a throwaway world, go back to the world list, click **Delete** on it, and press **Enter** a few times. Then click **Delete** again and click the red **Delete** button. **Pass:** Enter only cancels (**Cancel** is selected) and the world is still there. Only a click on **Delete** deletes it.
3. **Controls:** **Esc**, **Settings**, **Controls**. Change **Hotbar 1** to a different key and try it in the world. Change **Drop** to a key that is already used and read the line that appears (for example "Drop moved to G"). Leave the world, come back, and check the keys stayed. Then **Reset all to defaults** (it asks first; Cancel is selected). **Pass:** the changed key works, the clash swaps and says so, the keys are kept for every world, and reset puts everything back.
4. **Shield after a pause or a respawn:** raise the shield (right-click held), press **Esc**, then come back and right-click. Then `/kill`, respawn, and right-click again. **Pass:** each new right-click decides afresh whether to use the item or raise the shield. It does not repeat the old choice.
5. **Blight:** do Mission 2 (or look at a giant wheat field) and check the blighted crops are **ash grey** and half the size of healthy wheat.
6. **Respawn on dry land:** with no bed set, `/kill` and respawn. **Pass:** you stand on dry land near where the world started, never in the sea. Say where you came back.
7. **Satoshi's house:** in a new world, find the guide character's house near where you start. **Pass:** it stands on dry ground and shows up at once, and is not built on anything you placed.
8. **Bed respawn:** make a bed, sleep in it once, then `/kill` and respawn. **Pass:** you come back next to the bed, inside the room, not on the roof.
9. **Weather:** type `/weather storm`. Lightning comes about every 15 seconds on average (a roll every 8 seconds that lands a little over half the time) for 5 minutes. Say whether the thunder still makes you jump: it should rumble in after the flash and be softer when far. Then type `/weather clear` and check the rain stops. (To turn thunder down on its own, **Esc**, **Settings**, **Thunder** slider.)
10. **Chat completion:** type `/gi` and press **Tab** to get `/give`. Type `/give tor` and press **Tab** for `/give torch`, and **Tab** again for the next suggestion, **Shift+Tab** to go back. Then change **Esc, Settings, Chat text size** and see the chat get bigger. Leave to the lobby, open the same world again, and check the chat is empty.

**The re-check that failed last time (now fixed):** the held torch in F5. It is New 6 above.

**If it does not work:**
- Anything still looks like the old build: check the build number at the bottom of Settings says **v0.2.42**.
- Tab does nothing: Tab only completes lines that start with `/`.
- The furnace says "That can't be smelted or burned" for something you expected to work: the furnace smelts only raw iron, raw copper and tin (not meat, sand or cobblestone). Meat cooks on a campfire.

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
- Add a second job and tick it too.

**Look for:**
- The job and its tick are **still there** when you reopen it.
- **Confetti** bursts at the board the **first time you tick a job each game day**. That is the sign the place got a lift.
- Ticking a second job the same game day gives **no** confetti and no further lift. (A different board gets its own first tick. The game forgets which boards have been used when you leave and reopen the world, so confetti comes again after a reload.)

**What a short test can and can't show:**
- The lift to the place is **slow and small**. The tick lifts the place's "people" side at once, but the place's liveliness then eases up slowly: about two thirds of the way in 6 minutes, and only a few percent in the first minute. The lift itself fades over many minutes, and it only shows as stronger colour in a place that was **grey** or is already **very lush**. A normal green place sits in a middle range where the colour does not change at all.
- So in a short test **do not expect to see the colours change**. The confetti is the check. If the tester sees confetti on the first tick and none on the second, the board works.
- In Creative the lift cannot be seen at all (places never look grey there). Use Survival.

**The rules behind it:**
- A board holds up to **8 jobs**.
- Jobs come from a fixed list: Plant trees, Build walls, Build a roof, Dig a well, Light the way, Make a garden, Bring food, Gather wood, Clear the ground, Build a bridge, Fence the animals, Decorate.
- There is no free typing on a board.
- Anyone can add, tick or remove a job.
- The game does not check that the job was really done; the tick is the group's word.
- Ticking a job done gives the place round the board a small lift (the same as two guests at a feast), at most **once per board per game day**. Ticking and unticking again does nothing more.
- The crafting recipe: sticks down the left column, planks down the middle, sticks down the right column (3 high).

**If it does not work:** no confetti on the first tick? Check you have not already ticked a job on this board today (the game day, not real time), and that you are in Survival.

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
- If you are defeated, the off-hand item goes into the grave with the rest of your things (armour stays on you). Getting your things back from the grave puts it back in your off-hand.
- The off-hand is saved with the world.

**If it does not work:**
- **F** does nothing while you are painting a skin in the Workshop (the paint tool uses that key).
- Eating needs you to be hungry or hurt.
- Eating from the off-hand shows the same bite-by-bite picture as the main hand (New 10).

---

## Mission 6: The shield

**What it tests:** making a shield, raising it, blocking a hit, and its wear.

**Do:**
1. **Make one.** At a workbench (3 by 3 grid) put planks, iron ingot, planks along the top; planks, planks, planks across the middle; one plank at the bottom middle. That is a Y shape. (Or `/give shield`, but make one at least once.) Test items: `/give planks 6`, `/give iron_ingot`.
2. Hold the shield in either hand. Point at **open air or plain ground** and **hold right-click**. Count a quarter of a second (a quick "one-two"); the shield lifts in front of you.
3. Point at a **chest or a door** and right-click: that opens or toggles it, and the shield does **not** raise. Do the same at a **sheep or cow** with wheat or a Lead in your other hand: it takes the click (feeding, or the Lead) and the shield stays down.
4. With the shield up, **walk**, then try to sprint. Then **left-click** a block and a mob.
5. Type `/gm s`, then `/spawn brigand`, on open ground with **nothing behind you**. Raise the shield while the brigand walks at you from the front, and **stay where you are for about 10 seconds**. (The game has no zombies; the brigand is the enemy to use.) Watch the shield's durability bar after each clack. Once the shield is up, it stays up while you hold the button, even with the brigand under the crosshair.
6. Turn so the brigand is **behind** you, with the shield still up.

**Look for:**
- The shield is a wooden board with a grey rim and a round grey middle.
- A hit from **in front** does no damage and no push, and you hear a **clack**.
- The brigand stops against you and **you do not slide backwards**, even in open ground (this was the 0.2.41 bug: the brigand kept walking into you and slid you along).
- A hit from **behind** hurts as normal.
- You walk **slowly** (like sneaking) and cannot sprint.
- Left-click does nothing, and a block you were breaking stops cracking.

**The rules behind it:**
- A new shield has **336** durability.
- A blocked hit costs the shield **1 plus the damage it stopped** (rounded down). A brigand hit of 3 on Normal costs 4. One brigand in front wears it about once every half second, with one clack.
- The shield starts blocking after **0.25 seconds** (5 ticks) of right-click held. Letting go lowers it straight away.
- It blocks hits from the front half: within 90 degrees of where you look, flat across the ground. Height does not matter.
- It does not block lava, fire, falling, drowning or hunger.
- **A mob that is chasing you now stops at the edge of your body** (a hair short of touching) instead of walking into you. This holds for every hostile mob and for hunting hyenas, bears and sharks. It does not change a goat's charge, which is a deliberate ram.
- **Known, small:** a mob standing on a ledge **one block above** you can overhang you and nudge you across the drop (1.5 to 2.5 blocks) for a moment before it steps down. Once it is down it holds still. Not fixed.
- Hold right-click to raise it. A chest, a door, or a mob you can actually do something to (feeding, taming, a Lead, milking, shearing, riding, trading, or a command to your own pet, which takes an empty main hand) takes the click instead, and the shield stays down there.
- An enemy you cannot use anything on (a brigand, marauder or berserker) does not take the click, so facing one the shield goes up.
- Once raised, the shield stays up while you hold the button, whatever comes under the crosshair.
- With a shield in the main hand and a torch in the off-hand, right-clicking stone places the torch.
- Each press is decided once. Feed a cow with wheat in the other hand while you hold right-click: the cow is fed and the shield stays down for the whole press, even after the cow no longer wants wheat. Let go and press again and the shield goes up.
- In a multiplayer game the host's game decides what the shield blocks (both players on 0.2.42).
- A shield that is used up breaks and disappears.

**If it does not work:**
- Make sure you are not pointing at a chest, a door, or an animal or villager you can use something on.
- Hold the click for at least a quarter of a second before the hit.
- Check you are in Survival (`/gm s`): in Creative, nothing hurts you.
- Sliding backwards with the shield up is a bug in this build: say how far, whether anything was behind you, and whether the brigand was on higher ground.

---

## Mission 7: The lantern

**What it tests:** the lantern, light carried in your hand, and a lantern that drops when its block goes.

**Do:**
1. **Make one** at a workbench: an iron ingot, a torch, an iron ingot in a column (top, middle, bottom). Makes 1 lantern. (Or `/give lantern`.) Test items: `/give iron_ingot 2`, `/give torch`.
2. **Stand it:** right-click the **top** of a block.
3. **Hang it:** right-click the **underside** of a block (a ceiling).
4. Pick it up by breaking it, and check you get the lantern back.
5. **Drop test:** stand a lantern on a block and break the block under it; hang one from a ceiling block and break that block (New 11 has the full steps).
6. **Carry it:** type `/time set midnight` and go into a closed room or cave with no torches. Hold the lantern in your main hand and look around. Press **F** to move it to the off-hand and look again. Then try a torch in either hand.

**Look for:**
- A lantern is a dark iron frame with a warm yellow middle. It looks different standing and hanging (a hanging one has a little chain).
- A lantern whose block is broken **turns into an item** within about half a second, standing or hanging.
- A **lantern or a torch in either hand** lights the room round you, brightest next to you and fading with distance.
- Another player holding one lights the room round them too.

**The rules behind it:**
- A lantern gives light level **15**. A torch gives 14 and a paper lantern 12.
- It can be held, stood or hung. It is one item.
- A **standing** lantern needs a solid block under it. A **hanging** lantern needs a solid block, or another lantern, above it. A lantern clicked onto the side of a wall counts as standing, so with nothing under it, it drops at once. A paper lantern is a solid cube and stands on its own.
- Held lights light the area for you and other players, nearest first, up to 8 lights at once.
- Carried light is for looking only. It does not make mobs stop spawning and does not help crops grow.

**If it does not work:**
- If the room is not dark, the carried light is hard to see. Use `/time set midnight` and a sealed room.
- A lantern that was already floating in an old world stays until something next to it changes.

---

## Mission 8: Tool shapes

**What it tests:** every tool has its own shape in your hand and in its inventory icon, and the fishing rod casts a line.

**Type:** `/give iron_pickaxe`, `/give iron_axe`, `/give iron_sword`, `/give iron_shovel`, `/give iron_hoe`, `/give bow`, `/give fishing_rod`, `/give shears`, `/give shield`

**Do:** hold each tool in turn and look at your hand in the bottom right. Then press **F5** to see it in your hand from outside. Look at each one's icon in the inventory (**E**) and the hotbar. Then cast the rod at water (New 9).

**Look for:**
- The **sword** is **long and pointed**, with a crossguard wider than the blade.
- The **fishing rod**, held and not cast, shows a **thin line hanging from the tip with a small red float** on the end. **Cast**, the rod in your hand is bare, and the line runs from the tip to a bobber on the water.
- The pickaxe, axe, shovel, hoe, bow and shears are each different, not the same slab.
- Each tool's **icon** is the tool's own shape (not a flat square with a letter), in its material's colour.
- Another player holding a tool shows the same shape in their hand.

**The rules behind it:**
- Every tool type has its own shape, in first person and in third person.
- A tool made of a different material is the same shape in a different colour.
- The axe and the shovel can look alike at a glance (known; the shovel is flat and centred).
- **The fishing rod breaks blocks like a bare hand.** That is on purpose: any item that is not the right tool mines at hand speed.
- The cast reaches 12 blocks. A bite comes 5 to 15 seconds after the cast. Right-click again to reel in (a catch if there was a bite, nothing if you were early).

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
| `/give <item> [count]` | Gives any block or item by its name, as you see it in the game or its registry name, e.g. `/give spruce planks`, `/give deepslate`, `/give cobbled deepslate`, `/give granite`, `/give birch log 8`, `/give bedrock`, `/give site_board`, `/give shield`, `/give lantern`, `/give raw_iron 8`. Names can be several words: `/give satori helmet 2`, `/give satori armour` (the whole set). Only air, the piston's head, a Feast Marker, a Worn Path and a Blighted Crop can't be given. A typo gets "Did you mean ...?" |
| `/spawn <mob>` | Puts a mob 2 blocks east of you, e.g. `/spawn brigand` |
| `/we pos1`, `/we pos2` | Marks two corners at your feet |
| `/we set <block>` | Fills the marked box, e.g. `/we set wheat_stage_0` or `/we set air` (clears it) |
| `/we size` | Says how big the marked box is |
| `/trial list` / `/trial <id>` / `/trial off` | Lists trials, starts one, leaves one |
| `/time set day` / `/time set night` / `/time set midnight` | Daytime / night-time / the darkest time |
| `/sp` | Sets where you respawn to where you stand |
| `/kill` | Defeats you straight away (to test respawning) |
| `/weather clear` / `/weather rain` / `/weather storm` `[seconds]` | Clear sky (20 minutes) / rain / thunderstorm (5 minutes each), or for that many seconds (up to 3600) |
| **Tab** / **Shift+Tab** | Finishes a command or item name / goes back one suggestion |
| **F** | Swap what is in your hand with your off-hand |
| **F5** | Cycles first person, over the shoulder, behind you (see what you are holding and wearing). To look round yourself in F5, tick **Free-look** in Settings, then hold **Alt** and move the mouse |
| **F3** | Shows the debug overlay, including **FPS** (frames per second) |
| **E** | Inventory (also closes the furnace screen) |
| **Q** | Drop the held item |
| **Esc** | Pause menu (Settings is in it) |

---

## Feedback

At the end, write a short list like this, one line per mission, and hand it to the grown-up who looks after the game. A co-host writes its report in the shape its instructions give instead.

```
Build (bottom of Settings):
Ask first: controller or keyboard and mouse? / anything disappear?:
New in 0.2.42 (pass / fail each, with what you saw):
 1 Settings, pause menu scroll / 2 grave visible, gone when reclaimed / 3 /give names (spruce planks, deepslate, cobbled deepslate) /
 4 furnace screen (click to load, Shift whole stack, take back, refusal text) and planks of each tree burn /
 5 shield vs brigand (not pushed back, no damage, hurt from behind) / 6 torch in the fist in F5, both hands /
 7 icons (tools, armour, Plan, gate, slab, stairs, door, trapdoor, wall, fence post, spruce gate vs planks) /
 8 armour worn in F5, held, dropped, numbers 24 -> 15 -> 24 / 9 fishing bobber and line, bite, reel /
 10 eating (bites, crumbs, munch) / 11 torch and lantern drop / 12 cobbled deepslate (drop, stone tools, slabs) / 13 Creative bedrock:
Mission 0 older re-checks: log speeds / delete world / controls / shield after pause or respawn / grey blight / dry-land respawn / Satoshi's house / bed respawn / /weather / chat Tab and text size: worked / didn't. What happened:
Mission 1 Path: worked / didn't. What happened:
Mission 2 Wheat field: worked / didn't. What happened:
Mission 3 Site Board (confetti on first tick, none on second): worked / didn't. What happened:
Mission 4 Grey Quarry: worked / didn't. Words reached:   Time taken:
Mission 5 Off-hand (F swap / torch while mining / food / left hand in F5): worked / didn't. What happened:
Mission 6 Shield (recipe / raise after a quarter second / clack from the front / not pushed back / hit from behind / slow walking / no mining while up / durability): worked / didn't. What happened:
Mission 7 Lantern (recipe / stand / hang / drops when its block goes / carried light in either hand): worked / didn't. What happened:
Mission 8 Tool shapes (sword long and pointed / rod line and float / cast and bobber / icons / other tools different): worked / didn't. What happened:
Sound, lightning, lag: better / same / worse.
If laggy: where (e.g. mountains), and press F3 and read out FPS, Frame, Tick and Draws:
Best thing:
Anything broken:
```

## Links

- This guide: https://github.com/decented/axenstax/blob/main/docs/missions/latest.md
- Plain text of this guide: https://raw.githubusercontent.com/decented/axenstax/main/docs/missions/latest.md
- Player guide: https://github.com/decented/axenstax/tree/main/docs/player-guide
