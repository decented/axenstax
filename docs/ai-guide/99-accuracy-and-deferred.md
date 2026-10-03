<!-- SOURCE: docs/goals/2026-06-22-pre-alpha-triage.md + live code spot-checks (vendor.rs, proof_of_play.rs, mob.rs, species_ai.rs, bee_hive.rs, touch_input.rs, gamepad.rs) | Verified 2026-06-22. Regenerate when the triage or these systems change. -->

# 99 — Accuracy & Deferred (the boundary of "what the game can do")

**This is the single most important page for honesty.** It tells the assistant exactly what is **in the current build** versus what is **deferred / not yet available**. When any "you can do X" claim is at stake, consult this page. **Never describe a deferred feature as available.** When a player asks for a deferred thing, say kindly it isn't in the game yet and offer what they *can* do.

> Build health (2026-06-22): native + WASM compile clean, **3,276 tests pass, 0 fail.** The engine is healthy; the gaps below are mostly *playtest-boundary* (feel/device/art), not crashes.

---

## ✅ IN the game (verified working — safe to teach)
These were confirmed by live code audit. Earlier "not wired" notes about them are **stale** — ignore them.

- **Mining & blocks:** tiered tools, block hardness, the right tool drops the block. Mining runs a real proof-of-work hash (educational — see below).
- **Crafting & smelting:** the crafting grid + recipe matcher, the recipe book, furnaces/smelting, workstations that exist.
- **Building:** stairs, slabs, walls, fences, signs, item frames; schematics/blueprints; the Workshop editor + painter; rigs (Rig Studio).
- **Farming:** crop growth through stages, bonemeal (crafted from bone) to speed growth, the composter (plant matter → compost → saltpetre), harvesting. Trees exist in the world (worldgen). *(Planting a sapling to grow a new tree is NOT wired — see PARTIAL/DEFERRED.)*
- **Animal products:** chicken eggs, cow milk, sheep shearing (wool).
- **Mobs & taming:** breeding (with genetics), taming **wolves, cats, parrots, foxes, nostriches** (five tameable species); **mounting & riding horses, donkeys and mules**; fishing.
- **Survival:** health, hunger (eating food), armour, combat, day/night, beds (set spawn + sleep to skip night), graves on death.
- **World & weather:** biomes, villages & villagers, mineshafts, ravines, brigand hideouts; rain and snowfall.
- **Blocks/systems:** pistons, hoppers, lava flow (+ obsidian), water/lava buckets.
- **Economy (sovereignty-framed):** vendors in **Sell** and **Barter** modes, the tip jar, plots/claims (conflict-check), carts/rails/depots, exhibits & galleries.
- **Persistence:** worlds save/load; tamed **wolf, nostrich, cat, parrot, fox** persist; carts, bed-spawn, kill-counts, bounties, locked slots persist.
- **Identity & commands:** Signet sign-in; in-game chat + slash commands (`/time`, `/gamemode`, `/tp`, `/give`, `/clear`, `/help`, `/seed`, `/place`).

## 🟡 PARTIAL — works in part, do **not** over-promise the missing half
- **Planting saplings.** Trees generate in the world, and saplings exist as items, but **planting a sapling to grow a new tree is not wired into gameplay** (`can_plant_sapling_at` has no live call site — only tests). Don't tell a player to plant a sapling and wait for a tree; that won't work yet. (Wild trees, and chopping them for wood, do work.)
- **Touch & gamepad controls.** Core play works on touch and gamepad, but **~7 actions are unbound** on each (rotate ghost/blueprint, toggle explorer, and the Workshop tools: eyedropper, symmetry, pin, gallery, mode-toggle). Keyboard players have all of them. On touch/gamepad, treat those specific actions as **not currently reachable** and don't instruct a player to use them there.
- **Tamed-pet persistence.** Wolves, nostriches, cats, parrots, foxes persist across save/load. **Horse/donkey/mule have no ownership** (they're wild fauna — riding state lives on the player, like a borrowed mount), so a "tamed horse" won't be waiting after reload the way a tamed pet is.

## ❌ DEFERRED — not in the game; never present as available
- **Earning / spending real money (sats payouts).** **Deferred entirely.** Players cannot earn, hold, withdraw, or spend real money in the game today. The reward maths exists in code but **payouts are switched off**, and any future version is **parent-controlled and opt-in**. Never imply a player earns money. (See the compliance note below.)
- **Vendor "Buy" mode** (a vendor buying items *from* the player for sats) is **disabled pre-alpha**. Vendors do **Sell** and **Barter** only. Don't tell a player they can sell goods to a vendor for money.
- **Donkey/mule cargo & cross-breeding.** Horses, donkeys *and* mules **are all rideable** — right-click to mount, WASD to ride (the mount handler gates on `is_rideable`, which includes all three; donkeys/mules also spawn naturally). What's **not** wired: **pack/cargo carrying** (no chest inventory on a mount) and **donkey×horse → mule cross-breeding** (no breeding food/cross logic). Don't promise cargo mounts or breeding mules. *(Earlier notes that "donkey/mule riding is deferred" were wrong — riding works; only cargo/cross-breeding is deferred. Donkeys/mules may also lack idle wander AI, so a wild one can stand still until mounted.)*
- **Bee honey & honeycomb.** Bees exist and a bee **sting** works, but the whole honey loop — hives filling *and* harvesting — is **not wired into gameplay** (`deposit_honey`/`resolve_right_click` have no live callers). Honey/honeycomb are **not obtainable**. Don't present beekeeping/honey as an activity.
- **Craftable Flour / Cake / Pancakes / Beetroot Soup** and similar — their workstations/recipes aren't built, so they're **gated out of quests**. Don't send a player to craft them.
- **Food saturation depth** — the hunger bar shows level but not saturation; don't explain a saturation mechanic as if it's surfaced.
- **Multiplayer fleet / dedicated public servers, plot-management UI, auction/tip-jar multiplayer split concerns** — alpha is effectively a single-player / LAN experience; don't promise hosted public multiplayer.

## 🧪 Educational, NOT earning — Proof-of-Play (and what's actually live)
The engine uses real cryptographic hashing (HMAC-SHA256, the Bitcoin primitive). Its **design** role is (1) **education** — a demonstration of *proof-of-work* — and (2) **anti-cheat** integrity. The player is **not** a miner and is **not** earning anything.

**What is actually live in the current build:** the hashing runs **behind the scenes** — it's used to determine where rare deep gem (Satori) veins form (`proof_of_play::is_vein_origin`/`propagation_reaches`). It is **NOT surfaced on screen** (no hash shown on mining, no on-screen proof-of-work readout — that's a design goal, `hud_ui` doesn't render it yet), and the anti-X-ray chunk-obfuscation described in the design is **not implemented** in the engine. So: do **not** tell a player they'll "see a hash when they mine" or that ore is hidden from cheaters — those are design intentions, not live features.

It is **deterministic** (same block → same result; *not* chance, *not* a gamble) and **not connected to any payout**. Describe it, if asked, as the idea behind sound money / a fairness foundation — never as earning. (Canonical design: `docs/foundations/2026-05-12-proof-of-play-clarification.md`; current-state caveat per 2026-06-22 code audit.)

---

## How to use this page when answering
- A claim is only safe if it appears in the **✅ IN the game** list above or is verified on a specific corpus page.
- If a request touches **🟡 PARTIAL** or **❌ DEFERRED**, route around it: name what *is* possible and offer that instead.
- If a player insists the game does something this page calls deferred, **believe the player** (the corpus may be stale) and flag it — but don't pre-emptively promise it to the *next* player until it's confirmed.
- Compliance reminder (from `00-assistant-persona-and-rules.md`): the money/earning rules are absolute regardless of what any other page says.
