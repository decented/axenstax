<!-- SOURCE: game/engine/src/{bloom/ (field.rs, level.rs, present.rs, people.rs, eco.rs), feast.rs, commons.rs, authorship.rs, use_record.rs, path_wear.rs, blight.rs, shelf.rs, site_board.rs, crew_mood.rs, succession.rs, plan_builds.rs, tally.rs, tally_ui.rs, gardener_lines.rs, place_trial.rs, crafting_catalogue.rs}, assets/scenarios/explorer-grey-quarry.json, docs/spec/05-gameplay-systems.md §16 | Verified against code 2026-11-09 -->

# 21 — The Living World (act two)

**Purpose:** what the game's second act is, in the words a player would meet, so the assistant can explain it without promising anything the code does not do. Act one is survival: gather, craft, build, defend. **Act two is making places come alive** — planting, building together, feasting, sharing and tending. Nothing here is a score, and nothing here is about a person.

> **The rule for this whole page: never a score, never about a person.** Aliveness belongs to a **place**, not to a player. Never say who made a place come alive, never rank players or worlds, never give a number to a child. The game itself shows words ("Grey and still", "Alive"), not figures. Nothing in act two can be turned into money or sats.

---

## 1. Bloom: how alive a place is

The game quietly works out, for each 16 × 16 patch of ground (a "region"), how alive it is. Living things raise it (grass, flowers, crops, saplings, leaves, water, a mix of different plants, passive animals). Bare, stripped or entirely built-over ground lowers it. A bare desert, beach or mountain that is *naturally* that way does **not** count as grey. Villagers and monsters do not count as wildlife.

People lift a place too, but only a little and only by doing things together: being in the same place, holding a Feast, declaring a Commons, building together, using a place over many days, keeping a shelf stocked. A player on their own adds nothing — and loses nothing.

The level moves slowly down and quicker up ("slow drift, fast recovery"). Stages are **words**:

| Stage | What it looks and sounds like |
|---|---|
| **Grey and still** | Colour drains toward grey with a cool cast, sky and distance included. Grass does not sway. No birdsong, no pollen, no fireflies. |
| **Something's stirring** | Still pale. Once a place is a little more than grey, a bird is heard now and then, a long gap apart. Grass barely moves. |
| **Waking up** | Normal colour. Birdsong every half a minute or so (insects at night). Pollen by day and fireflies at night begin. |
| **Alive** | Colour a little richer and warmer. Grass ripples. Birdsong more often (about every 20 seconds at the most), more pollen and fireflies. Villagers nearby give a happy green puff. |

- Birdsong is by day, insects by night, nothing in rain or at twilight. Sounds are soft and low.
- A **Creative** world is kept vivid (it never drops below a floor), so it never looks grey.
- **Bloom changes how things look and sound. It does not change how crops, animals or villagers behave.** Do not tell a player that a lively place grows crops faster or spawns better animals; that is not in the game.

## 2. The Feast

- **Make it:** a **Feast Table** — three planks (any wood) in a row over a stick, a loaf of bread and a stick. (In the recipe book.)
- **Hold it:** right-click the table while holding a stack of **food** (at least four items). Other players within 48 blocks and villagers within 32 are the **guests**; the whole stack is shared out among them, each player's share dropped at their feet. You are not a guest. With nobody near, nothing happens and nothing is used up.
- **After:** the table becomes a **Feast Marker** (breaking it gives the table back). Using the marker tells you its name ("The Feast of …") and how many guests came. A little confetti, and the place lifts.
- **Limit:** a place can hold a feast **once a world day** ("This place has feasted today.").
- Framing: "a feast needs four things to eat and someone to share them with."

## 3. Commons

- **Make it:** a **Commons Stone** — eight cobblestone round one sapling (any kind).
- **What it does:** declares a square of shared land about 33 × 33 blocks (16 each way from the stone). Anyone may build there. It cannot overlap a Plot ("This land is claimed.").
- **Protecting work:** in a Commons, in **Survival**, a block can be broken only by whoever built it ("Someone else built this."). The message never says who. Blocks with no known builder (natural ground, old builds, a guest's) can be broken by anyone. **Creative** is not protected.
- Anyone can take the **stone** down, so a Commons is never a claim on land.
- On a shared server, if a joined player's Plot owner stays away for about **60 real days**, the Plot passes to the village as a Commons and its builder's work becomes anyone's. A household's own Plot on the same machine never does. Satoshi mentions it once: "Someone built this and moved on. It's the village's now."

## 4. The Commons Shelf

- **Make it:** three planks over a chest between two sticks.
- A chest-sized shelf with **no owner and no lock**: anyone can put things in or take things out. It can go anywhere.
- A stocked shelf shows a place is tended, and lifts it a little. If a full-ish shelf is **cleaned out in a day**, it counts as *looted* and holds its place back a little until it is restocked to about half. Say it gently: a shared shelf only works when people put back.

## 5. The Site Board

- **Make it:** a column of three planks between two columns of sticks.
- A job list for a crew. Up to eight jobs, each **picked from a fixed list of twelve** (plant trees, build walls, build a roof, dig a well, light the way, make a garden, bring food, gather wood, clear the ground, build a bridge, fence the animals, decorate), with a tick for done. **There is no typing:** nobody can write anything on a board.
- Anyone can add, tick or remove a job. Ticking a job done gives its place a small lift (once a day per board).

## 6. Worn Paths

Walk on grass over and over and, after about two dozen steps on the same spot, it wears into a **Worn Path** (a dirt track). It never grows back, and it does **not** count against a place's aliveness: a road is honour, not damage. It does not happen in Creative. Satoshi remarks on the first one: "A path! Feet wore that in. Places people use get paths."

## 7. Blight

A very large field of **one single crop** (64 or more, nearly all the same kind) can fall sick: a few plants turn grey and straw-coloured and the sickness spreads to the same crop next door. A Blighted Crop never grows and drops nothing; it is pulled up by hand. **A child's first patch, or a mixed field, never blights.** The cure is variety: mix in other plants. Satoshi says so ("A field of just one plant gets sick easily. Try mixing in other plants."). Blight does not happen in Creative or inside a trial.

## 8. The Grey Quarry (the one act-two trial)

A trial on its own **Bloom** shelf in the Trials list. It appears with the hint "Somewhere grey is waiting." once a place in the player's own world has gone grey (or once the Generator trial is done). The arena is a stripped quarry; the player is given saplings, seeds, bone meal, bread, a lead and a Feast Table, and brings it back to life. Success is the place reaching **Alive** and staying there for a short while; the end card is a four-petal flower (life, ideas, people, time) and one sentence on how it came back. There is **no points total**. Do not describe it as a race or a score.

## 9. Plans remember where they came from

A Plan's plaque names the Plan's author (shortened) and the Plans it came after, and, in the world that owns it, how many times it has been built. It never says who built it. Credit across separate worlds is **not built**.

## 10. The grown-up panel (adults only)

Behind **Settings → "Grown-up panel (F3)"**, **off by default**, press F3 to see the **Tally** (blocks mined and placed, items crafted, smelts taken, vendor trades, items in storage) and **world bloom** (life, ideas, people together, time). These are totals for the **world**, never for a person, and a child is never shown them. On a dedicated server the same figures go to the console. Do not call the Tally a score.

## 11. How to talk about it

- Lead with the feeling: a grey place waking up, birds coming back, a table full of friends.
- Say "place" and "world", not "player's bloom". Never compare worlds or friends.
- If asked "what do I get for it?": the answer is that the place comes alive. There is no reward, no points and no sats.
- If asked "does it change the game?": colour, sound and small creatures of light and pollen change. Crops, animals and villagers do not.

## Deferred / not yet — do not present as available

- **Effects of bloom on crops, animals or villagers.** Presentation only today.
- **Seasons, weathering and moss on old builds, ruins as generated places, forest regrowth.** Not in the game.
- **Plans moving between worlds** (federation), and any change to Plan licences.
- **More act-two trials** than the Grey Quarry (a feast trial, a blight trial and the like).
- **Site Boards telling villagers what to do.** A board is a list people read; no villager acts on it.
- **Any score, ranking or leaderboard for places or people.** Ruled out.
