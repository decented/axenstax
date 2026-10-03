# Test sheet — Wind, Copper & Electricity (2026-09-07)

**Build:** v0.2.26
**Needs:** one machine for most of it; a second machine (or a second player
joining your hosted world) for the multiplayer section. Survival or Creative
both fine unless a step says otherwise.

Design: `docs/superpowers/specs/2026-09-07-wind-copper-electricity-wave-design.md`.
Player guide: **[Electricity](../player-guide/electricity.md)**,
**[Blocks & Mining](../player-guide/blocks-and-mining.md)**.

**How to report:** play, then jot a note per line — ✓ works / ✗ doesn't /
🤔 felt off.

## 1. Copper — the survival chain

Copper Ore now generates underground for the first time — before this build it
only existed via `/give`.

- [ ] Start a **fresh Survival world**, no `/give`. Dig down to roughly Y 30–69
      (mid-depth stone, above the deepslate line) and find **Copper Ore** — a
      copper-flecked stone block, distinct from Iron/Coal/Nitre/Brimstone.
- [ ] Mine it with a **wooden** pickaxe first — it should break with **nothing
      dropping** (wrong tier).
- [ ] Mine it with a **stone+** pickaxe — it drops raw **Copper**.
- [ ] Smelt Copper in a Furnace → **Copper Ingot**.
- [ ] Craft **Cable**, a **Lamp**, and a **Lever** from the recipe book, and
      wire a working circuit — all from things you dug and smelted yourself,
      no `/give`.
- [ ] 🤔 How long did the whole dig-to-lit-lamp loop take? Did it feel like a
      fair, findable ore, or too rare / too common?

## 2. Windmill

New free-running power source — turns in real wind, no fuel, no winding.
`/give windmill` if you don't want to craft one (recipe: Canvas / Plank /
Stick ringing Copper + Iron — flag if it feels wrong).

- [ ] Place a Windmill **at ground level** (sea level or below), wire it to a
      Lamp. Look at it (or check **F3**) — it shows a wind word (calm / light /
      fresh / strong / gale) and whether it's turning, still, or blocked.
- [ ] Watch it for a couple of minutes. At sea level it should be
      **intermittent** — turning sometimes, still other times, tracking the
      wind word.
- [ ] Build a **roof** one block above the sails (or wall in three of its four
      sides) — the mill should **never turn**, however windy it gets, and the
      hover label should say it's blocked ("needs open sky"), not just still.
- [ ] Build a **second Windmill up high** — a hilltop, or a tower ~30 blocks
      above sea level — wired to its own Lamp.
- [ ] 🤔 **Feel question:** on a clear day, does the hilltop mill basically
      **never stop**, while the sea-level one comes and goes? That's the
      intended "build it high" lesson — but it makes altitude a hard on/off
      rather than a gradient. **Does that feel too binary?** (If so, the fix is
      a single constant — the altitude cap — not a redesign.)
- [ ] A **thunderstorm** should make even a sea-level mill turn much more
      reliably than a clear day. Wait for a storm (or `/scenario catch-the-wind`,
      see §4) and compare.

## 3. Electricity bug fixes (this build)

- [ ] Wire a **Battery** to a Lever + Cable, charge it, then flip the Lever
      off. The Battery should hold the circuit for a while and then **run
      down for real** — not stay lit forever. (Previously it could top itself
      back up off its own wire and never run out.)
- [ ] Open your inventory / hotbar and hover any electricity block (Lamp,
      Cable, Water Wheel, Windmill, etc). The label should read plainly (e.g.
      "Water Wheel"), **never** with a namespace prefix like "Electricity:…".

### Multiplayer (needs a second player)

- [ ] **Host** a world. Have the **joining player** place a Logic Gate, a
      Mirror, or anything with a facing. On the **host's** screen, does it
      face the way the joiner actually placed it (not always north)?
- [ ] Joining player builds a small circuit (Lever → Cable → Lamp) entirely on
      their own placements. Does it light up for **both** players, and does
      the host's own world actually simulate it (not just render it)?
- [ ] Joining player **right-clicks their own Lever** to flip it. Does the
      lamp respond for **everyone**, including the host?

## 4. Trials

Seven new Trials on the Challenge Board (**J**) — trial count is now **45**
Explorer Challenges. `/scenario <token>` jumps straight to one.

- [ ] `catch-the-wind` — place a Windmill on the hilltop platform under a
      **locked storm**; it should turn reliably and light a lamp.

The other six have **not been played end to end by a solo build pass** —
please check every one of these explicitly, they need a real playtest:

- [ ] `copper-rush` — the ore face gives **≥6 Copper Ore actually reachable**
      with the **stone pickaxe** the kit provides (not blocked by the arena
      geometry); mine, smelt, and craft a Cable through to completion.
- [ ] `mill-race` — the waterfall in the arena **actually turns** the Water
      Wheel once you place it where the kit intends (not a still pond by
      accident); get it turning and light a lamp.
- [ ] `fresh-coat` — opens straight into a **Workshop** (paint bench, avatar
      mannequin) without leaving the trial. Paint the mannequin's skin and
      pin it on.
- [ ] `follow-the-plan` — lay the given hut plan and **build it along, block
      by block**. **Important:** the plan dialog also offers a "build it for
      me" auto-build button — that does **NOT** count and won't complete the
      trial. Only placing the blocks yourself does. Flag if that's confusing.
- [ ] `bouncer` — build a creature in **Rig Studio (Y)**, set its motion to
      **Bounce**, and Spawn it.
- [ ] `suggestion-box` — send an idea with `/idea <text>` **inline, in one
      line** (text right after the command). A bare `/idea` with no text opens
      a separate composer instead and does **not** complete the trial.

## Questions for Axolittle

1. Copper Ore: fair rarity/depth, or should it be easier/harder to find?
2. Windmill: is the hilltop-vs-sea-level difference obvious and satisfying, or
   confusing? (See the binary-altitude question in §2.)
3. Did `follow-the-plan`'s "build it for me" button trip you up before you
   found the real objective?
4. Anything in the electricity tier that still feels broken or unfair?

## Notes
