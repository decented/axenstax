# Villages & Villagers

Villages are little procgen settlements scattered across Plains and Forest biomes — roughly one per 32×32 chunk grid cell. Each one has a handful of houses, a well, a campfire, and 3–6 villagers who live there. Some villages have a Knight patrolling as their defender.

## Finding a village

Walk in any direction in a Plains or Forest biome. Watch for:

- A cluster of small cobblestone houses with planks roofs.
- A 3×3 cobblestone well with a 1×1 water source in the middle.
- A lit campfire on a cobblestone hearth.
- A smoke pillar rising above the campfire when leaves are burning.

The first time you get within ~24 blocks of a village, you'll see a toast: **"You've found a village! (gx, gz)"** — the numbers are the village's grid cell. A small **reputation panel** appears in the bottom-left of your screen showing the village's coordinates and your current standing.

### Spawning near one

When creating a world (or loading one), use the **Spawn dropdown** above the world list. Pick **Near a village** and the engine searches outward for the closest village and drops you on its hearth. See **[World Creation & Spawn](world-creation-and-spawn.md)**.

## Houses

Each house is a 3×3×4 cobblestone box with:

- A **planks floor** (oak).
- A **bed** in the back corner. **Right-click at night** to sleep — skips to morning, full heal, sets your respawn point. The village's beds aren't claimed by villagers; any one is fair game.
- A **door knocked into the inward-facing wall** so the door points toward the village centre.
- A **ceiling torch** that lights the interior at night.

Hostile mobs **don't spawn inside houses with doors + roofs**. Use a village as a safe overnight base.

## The well

A 3×3 cobblestone rim around a 1×1 water source, with a low wall. Drink animations / water bucket use will land here eventually. For now it's:

- A convenient water source for **adjacent crop irrigation** — till the dirt right next to the rim and your wheat grows fast.
- Decorative + landmark.

## The campfire

A 5-block cobblestone cross hearth with a lit campfire on top. It's lit on world-gen — fuel is generous so it'll burn for a while before going out.

- Cook your own raw meat on it (right-click while holding raw meat) — see **[Food & Cooking](food-and-cooking.md)**.
- The smoke pillar attracts hostile mobs and draws them into the Knight's defensive range. The village kills its own intruders.

## Villagers

3–6 villagers spawn per village (one per house). They're passive humanoid NPCs in brown robes. They wander, look around, and respond to your right-click.

### Professions

Each villager binds to a **profession** by walking near a workstation block. The profession determines what quest they offer:

| Profession | Workstation | Status |
|---|---|---|
| **Farmer** | Tilled Soil | ✅ Live |
| **Cook** | Campfire | ✅ Live |
| **Carpenter** | Workbench | ✅ Live |
| **Blacksmith** | Furnace | ✅ Live — quest pool covers iron + coal Fetch lines |
| **Builder** | Drafting Table | ⚠️ Profession assigned but no quest pool yet — Builders offer **commissions** (slot a Plan + materials, pay a fee, NPC builds for you); the commission UI lands in a follow-up update |
| **Scribe** | Bookshelf | ⏳ Waiting for Bookshelf (coming soon) |
| **Miller** | Mill | ⏳ Profession exists, but the Mill workstation isn't functional yet |
| **Baker** | Oven | ⏳ Profession exists, but the Oven workstation isn't functional yet |
| **Brewer** | Aging Rack | ⏳ Profession exists, but the Aging Rack workstation isn't functional yet |

Villagers default to **Unemployed** until they wander near a workstation. The first villager near a Workbench claims it and becomes a Carpenter; the next villager has to find a different workstation. **Claims are sticky** — a villager stays in their profession until the workstation block is destroyed.

If you tile up some grass next to an unemployed villager, they may convert to Farmer within ~2 seconds.

### Talking to a villager

**Right-click** within ~4 blocks. The dialogue overlay opens, showing:

- The villager's label: **"Villager #N, Profession"**.
- The current **quest offer** (if they have one), or **"No quest right now."** if they're Unemployed.
- A short **gossip line** in quotes — flavour text that changes daily (e.g. "Strange tracks in the woods last night.").
- Three buttons: **Accept** / **Decline** / **Close**.

Accept the quest and the dialogue closes; the quest is now active and tracked. Decline it and the villager won't offer you another quest for **5 minutes**. Once you've accepted, talking to the villager again shows a distinct **"in progress"** dialogue (with your live progress) until the quest is actually done, at which point it switches to a **"ready to turn in"** dialogue. See **[Quests & Reputation](quests-and-reputation.md)** for the full quest loop.

### Gossip

The gossip line is **deterministic per (villager, in-game day)** — so the same villager says the same thing all day, but a different line tomorrow. Profession-flavoured gossip is more likely from professional villagers: a Cook is more likely to mention campfire tips, a Carpenter is more likely to mention sticks-and-planks lore.

Gossip also surfaces raid warnings — a live villager may tell you "Bandits coming for this village tonight" when a raid is brewing, alongside flavour and profession-flavoured hints.

### Hitting a villager

The **first hit on any villager** in any 30-second window does no damage. A toast pops up: *"Careful — that's a villager. Hit again to attack."* If you hit again within the window, the next swing damages normally. This is the **anti-grief gate** — gives you a chance to bail out of an accidental swing.

If you actually kill a villager, you lose **25 reputation** with that village (one penalty per 30 s, so a rage-loop costs once). A toast confirms: *"The villagers saw that. Your standing here just dropped."* See **[Quests & Reputation](quests-and-reputation.md)**.

## Reputation

Each player has a per-village reputation score (a number from -100 to +100). Tiers:

| Tier | Range | What it changes |
|---|---|---|
| **Hostile** | ≤ -50 | Villagers refuse to interact. |
| **Wary** | -49 to -10 | Quest sats reward × 0.75. |
| **Neutral** | -9 to 9 | Baseline. |
| **Friendly** | 10 to 49 | Sats reward × 1.10. |
| **Beloved** | ≥ 50 | Sats reward × 1.25 + bonus quests. |

The HUD reputation panel (bottom-left when in a village) shows your tier in colour: red Hostile → amber Wary → grey Neutral → green Friendly → gold Beloved.

Reputation **decays one point per in-game day** toward zero — past mistakes fade, but so do past triumphs. Keep questing if you want to stay Beloved.

## Tips

- Villages are the safest place to sleep through your first night. The Knight (if there is one) will kill mobs that wander in.
- A handful of Carpenter quests early on builds your reputation cheaply (sticks + wood are easy to get).
- **Don't till every grass tile in the village** — you'll knock the Farmer's profession claim out by destroying surrounding tiles in the chain.
- The Farmer's tilled soil is fair game to harvest from — they don't claim the plants, just the workstation block. Treat the village garden as a shared resource.
- If you find a village without a Knight, the village has fewer than 3 employed villagers or fewer than 5 houses. Help its villagers find jobs (place workstation blocks) and it'll get one within a minute.
