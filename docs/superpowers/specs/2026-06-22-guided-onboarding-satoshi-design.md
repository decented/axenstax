# Satoshi — The Optional In-World Guide (Onboarding Design)

**Replace formal tutorials with a charming, optional, in-world guide whose currency is *fun*, not instruction — and who happens to be the founder figure of the sound-money world.**

- **Date:** 2026-06-22
- **Status:** 🌱 **LIVING / DESIGN** — captured from a design conversation with the owner; informed by `docs/research/2026-06-22-guided-onboarding-precedents.md`. Not a frozen spec; concrete enough to build an MVP from.
- **Companions:** the research doc (evidence base), `docs/vision/genesis-founding-myth-long-run.md` (Satoshi as founder figure), and the earlier "Tier-1 fun leverage" findings (persistent goals, reward juice — Satoshi is the human face over that scaffolding).

---

## 1. The problem
Axo (our playtester) said it plainly: **formal tutorials are boring — "I just want to play in the world and see what happens."** And our guiding principle says the incentive should be **fun itself**, never an extrinsic reward (no "do this, earn sats"). So we need onboarding that doesn't *feel* like onboarding: a way to draw a player into the game's activities through curiosity and delight, that they can also completely ignore.

## 2. The solution in one line
**An optional, charming, in-world character — Satoshi — who notices you, offers a friendly nudge ("hey, are you hungry?"), shows you things by doing them beside you, and can always be waved off or sought out later.** He teaches by *being* part of the world, not by stopping the game to explain it.

## 3. Who Satoshi is
- **The founder figure *and* the guide — one character.** Onboarding and lore are the same person: the mysterious founder of the sound-money world (ties to the Genesis founding myth). He teaches you to play *and* quietly seeds the world's story — never with a lecture.
- **Warm, low-pressure, with a real personality beyond helping** (the Isabelle/Toriel/BT-7274 lesson). He's someone you'd *want* to bump into, not a tip overlay. Tells you to take a break, celebrates your wins, has opinions and a history.
- **He genuinely likes and respects you** (reciprocal liking) — encouraging about *your* choices, never flattering or fake.
- **He cares — and that's his in-fiction reason to help** (the Toriel model). "Are you hungry?" *is* the design: care as the motive for guidance. A new arrival in a frontier world; of course someone checks you're fed.

## 4. Design pillars (each traced to the research)
1. **Pull-first and always optional (Isabelle).** Satoshi offers; the player disposes. "I'm OK, I'll come back later" / ignore him entirely is *always* valid and never penalised. He escalates help only when *you keep asking*.
2. **He lives somewhere you can choose to visit.** A little house — knock on the door when *you* want guidance. Pull-on-demand beats push. (He may also wander/appear gently — see the re-engagement rule in §6.)
3. **Co-presence, not control (never seize input).** Satoshi demonstrates *beside* you — he plants a seed, you watch, you try — he never grabs the controls or locks your movement.
4. **Teach by doing-in-context, with meaning.** He introduces a thing *only when you can act on it* and attaches a why ("eat this — you'll mine longer before you tire"). No front-loaded explanation.
5. **The world teaches first; Satoshi fills the gaps.** Lean on environmental signposting (block/animal/crop placement, the day/night rhythm). Satoshi is the *soft, human layer* over a world that already nudges — not the primary teacher.
6. **Minimal reading (the kids constraint — biggest one).** Short lines, emotes, pointing, demonstration, icons — *not* paragraphs. An 11-year-old who hates tutorials will skip text; never gate progress on reading.
7. **Choice → tiny goal → gift, not a checkbox.** "Hungry? Want to meet the animals, or try growing something?" → a small achievable target → a useful *welcome gift* (seeds, a tool), framed as a kindness, with **no "tutorial complete" pop-up.**
8. **Show the locks before the keys (optional content only).** Let the player *see* animals, a crop, a far-off structure before Satoshi hands over the means — the unlock feels earned. Never gate mandatory first-session progress this way.
9. **Fun is the incentive (SDT: competence, autonomy, relatedness).** Every nudge should make the player feel capable, free, and connected — not obligated. That's what brings them back tomorrow.

## 5. The first-session shape (illustrative, not prescriptive)
You spawn. You're free to wander immediately (Axo gets his wish). Within a little while, Satoshi ambles over — warm, unhurried:
- *"Oh — hello! New here? You look a bit peckish. Here, this'll keep you going."* (Hands you food; you learn eating exists, in context, as a gift.) You can say **"Thanks, I'm good — I'll explore"** and he tips his hat and leaves.
- If you're curious, he offers a **choice**, not a script: *"Want to meet the animals down the way, or have a go at growing something?"* Either path is a tiny, concrete, optional goal.
- He **shows by doing** — tills a patch, drops a seed, waits with you for it to sprout — then lets you do the next one.
- When you wander off, he doesn't chase you. He's **at his house** if you want him. He might wave next time you pass — *once*, not a nag.

## 6. Anti-patterns — hard guardrails (from the research)
- **Not Navi.** No repetitive, urgent, attention-grabbing interruptions. Define a **frequency budget**: Satoshi initiates *rarely*, never repeats a tip you've acted on, and goes quiet if waved off. (Miyamoto called Navi's nagging a "major weakness" — we don't repeat it.)
- **Not Fi.** Nothing unskippable; never stop play to reiterate what the player just did. All guidance dismissable on a single input.
- **Not Flowey/Clippy.** Never condescend. Respect the player's intelligence and agency, always — especially with kids.

## 7. How this leverages what's already built
This is *delivery*, not a new system — it's the human face over assets that exist (ties to the earlier Tier-1 "deliver what's built" findings):
- **Quests / challenge board / bounties** already exist — Satoshi becomes their warm, optional voice, and they get the **persistence + visible "what next?"** they currently lack.
- **Villagers** exist — Satoshi is a special villager; the co-presence demo reuses villager behaviour.
- **Existing activities** (eat, tame, plant, mine) become the things he nudges toward — no new content required for an MVP.
- **Bonus — it flushes bugs for alpha.** A gentle guided path naturally walks testers through the key mechanics, surfacing what's broken (directly serving "I don't know the bugs until they're tested").

## 8. Parked (not now): the AI-driven Satoshi
The idea of letting a player plug in an **AI API key** so Satoshi can converse dynamically is **deferred** — the **child-safeguarding risk is a nightmare** (unbounded AI talking to kids). Noted as a far-future possibility only behind serious safeguarding; **do not build.** The MVP Satoshi is fully authored/scripted.

## 9. Open decisions (for us + the playtest)
- **Delivery medium** with no voice budget: short captioned lines vs icon/emote-driven vs hybrid? (Lean low-text.)
- **Where he lives / how he appears:** house-to-visit (pull) vs gentle contextual wander vs both — and the exact re-engagement frequency that stays the right side of "nagging."
- **Does NPC co-presence feel collaborative** (not a cutscene) to an 11-year-old? **Untested — this is the key playtest question.**
- How much Satoshi *leads* vs merely *suggests*; his voice/personality specifics; his look and his little house.

## 10. Scope & phasing
- **MVP (pre-alpha-friendly):** a scripted Satoshi who appears once, offers the "hungry?" gift + a 2–3 option nudge, demonstrates one activity beside you, is fully dismissable, and lives in a findable house. Reuses villager + quest + challenge assets. Low new-system cost.
- **Later:** richer branching nudges, deeper tie-in to the founding-myth lore, more activities, the "show the locks" teases for optional content.
- **Gate:** like everything, this needs the **voluntary-player fun test** — does an un-coerced kid find Satoshi charming-not-annoying, and does the world pull them in? The co-presence question above is exactly what that test answers.
