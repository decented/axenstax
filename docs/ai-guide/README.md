# AxeNStax AI Guide Corpus

**A structured, LLM-readable knowledge base about the AxeNStax game world — built so an AI assistant can discuss the game, answer player questions, and generate tutorials on the fly, accurately and kid-safely.**

- **Created:** 2026-06-22. **Verified against engine code on:** 2026-06-22.
- **Status:** Living. **Regenerate from the code whenever the game changes** (see Accuracy Rule).

---

## Who consumes this
- This corpus is a knowledge base written **for an LLM to read** — dense, factual, structured (headings, tables, explicit facts), not marketing prose. It exists so a **future AI assistant** can discuss the game and generate tutorials accurately and kid-safely.
- **Status (2026-06-23): there is no live AI assistant wired to this corpus.** The original intended consumer — a website **"Games Master"** chat widget — has been **removed from the project; do not reintroduce or suggest it.** The corpus stands as accurate, ready groundwork for whenever a suitable assistant is built.
- The scripted **Satoshi** onboarding guide (`docs/superpowers/specs/2026-06-22-satoshi-onboarding-foundation-spec.md`) is a separate, **fully scripted** MVP — not an AI consumer of this corpus (an AI-driven Satoshi is parked for child-safeguarding).

## How to use it (for the assistant)
1. **Load `00-assistant-persona-and-rules.md` first** — it is your behavioural contract (voice, kid-safety, the Bitcoin/earning rules, how to generate tutorials). It overrides anything else if they conflict.
2. Answer from the **knowledge pages (01–18)**. They are code-verified.
3. Whenever a "you can do X" claim is at stake, consult **`99-accuracy-and-deferred.md`** — the boundary between *in the game* and *deferred*. **Never promise a deferred feature.**
4. To teach, use **`20-tutorial-seeds.md`** — grow a seed into a live, one-step-at-a-time tutorial; don't read a script.

## The Accuracy Rule (non-negotiable)
This corpus is **verified against the engine source** (`game/engine/src/`). It is the *current* game, not the vision docs and not Minecraft. Three commitments:
- **Only describe what the code actually does.** If it's not in these pages, treat it as not available.
- **Regenerate when the game changes.** When a mechanic ships or changes, re-verify the affected page against the code and update it (and `99-accuracy-and-deferred.md`). Each page carries a `<!-- SOURCE: … | Verified … -->` header naming the files it was checked against.
- **When in doubt, omit or flag.** A wrong promise to a child is a real harm. Accuracy beats completeness.

## The compliance core (summary — full version in `00`)
- **Never** pitch earning: no "earn Bitcoin", "mine Bitcoin", "make money". Lead with **sovereignty / freedom / building**.
- In-world, **money is just money** (old money = diamonds; sound money). Don't say "Bitcoin" unless a player explicitly asks about the real-world link — then keep it educational and **defer real money to a parent**.
- **Proof-of-Play is educational + anti-cheat, never earning**; real-money payouts are **deferred** and parent-controlled.
- **Kid-safe, UK English, never over-claim safety.**

---

## Index
| Page | Purpose |
|---|---|
| `00-assistant-persona-and-rules.md` | **Read first.** The assistant's behavioural contract: voice, kid-safety, Bitcoin/earning rules, how to generate tutorials, refusal boundaries. |
| `01-world-and-lore.md` | The world's spirit & light backdrop (founding-myth as *tone*, show-don't-preach). The Genesis Block is the one real in-world artifact. |
| `10-controls-and-movement.md` | Keyboard/mouse bindings, movement physics, camera; touch/gamepad gaps. |
| `11-mining-tools-blocks.md` | Tool tiers, the tier-gate ladder (what mines what), block hardness, drops; Proof-of-Play (educational). |
| `12-crafting-and-recipes.md` | How crafting works + a verified sample of real recipes; smelting (furnace) vs cooking (campfire); workstations. |
| `13-building.md` | Building blocks & shapes, signs/frames, schematics/blueprints, the Workshop editor + painter, rigs. |
| `14-mobs-animals-taming-breeding.md` | The 29-mob roster, hostile vs passive, taming (five species), breeding & genetics, mounts, drops, persistence. |
| `15-farming.md` | Growable crops, planting & growth, bonemeal, composter, animal products; what's *not* growable. |
| `16-survival-combat-health-hunger.md` | Health, hunger/eating, armour, combat, day/night danger, death/graves, game modes. |
| `17-world-biomes-structures.md` | Biomes, world generation, structures (villages/mineshafts/ravines/hideouts), world creation, the Genesis Block. |
| `18-economy-and-vendors.md` | Vendors, tip jar, plots, bazaar, market hubs, auctions, rail freight — **sovereignty-framed, notional sats, no real earning.** |
| `20-tutorial-seeds.md` | A library of tutorial seeds the assistant expands on the fly (first 15 minutes, shelter before night, tool ladder, farming, taming, cooking, claim & trade, the Genesis Block). |
| `99-accuracy-and-deferred.md` | **The boundary.** What's IN the game vs PARTIAL vs DEFERRED. Consult whenever a claim is at stake. |

*(Numbering leaves room: 0x = foundations, 1x = gameplay knowledge, 2x = teaching, 9x = meta/accuracy.)*
