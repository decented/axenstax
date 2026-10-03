# Historical Pivot — Long-Run Master Design

**Status:** Vision-level design, brainstormed 2026-05-22. Anchors a 7-spec decomposition. Sub-foundation 0 (Naming Pass) and Sub-foundation 1 (Drop-Economy Migration) are fully specced at `docs/foundations/2026-05-22-historical-pivot-naming-pass.md` and `docs/foundations/2026-05-22-historical-pivot-drop-migration.md`. Subs 2-6 are stubbed below + Sub 7 (Miller/Baker/Brewer professions) appended as a parallelisable add-on; full foundation specs get brainstormed when each lands in the build queue.

**Companion to** (sits alongside as the fourth long-run pillar):
- [`farming-economy-long-run.md`](farming-economy-long-run.md)
- [`economies-long-run.md`](economies-long-run.md)
- [`build-schematics-long-run.md`](build-schematics-long-run.md)

---

## Statement of intent

AxeNStax pivots from a fantasy threat aesthetic (zombies, skeletons, dragons) to a **medieval-realistic** one. Nighttime threats are **humans** (organised brigands) and **animals** (territorial wildlife). The economic and exploration loops (mining, farming, villages, market stall, plans, Charter, sats) are unchanged — only the threat composition + the *names* shift.

Selective fantasy elements remain only where they're economically or narratively load-bearing.

---

## The naming principle

**Pick the name a medieval person would have used.** If Mojang happened to land on the same English word for the same thing (Furnace, Pickaxe, Carpenter, Sword), that's coincidence — both arrived at the right historical word. We diverge only where Mojang chose something quirky or coined and the proper historical term is different.

This is an *identity* principle, not an IP-defence principle. The IP-distance benefit is a side effect, not the driver. Sub-foundation 0 (Naming Pass) applies the principle to the current codebase.

---

## Kept fantasy

- **Nostrich** — purple mascot mob with the Vow curse + 21×-yield Bitcoin-egg. Mascot identity + Vow mechanic + recipe economy would all break if removed; replacement cost exceeds value.
- **Satori** — the orange Bitcoin gem mined from deepslate. Core to the Proof-of-Play fiction. Coined word; orthogonal to the medieval theme.

Both remain unchanged by this pivot.

---

## Retired fantasy

All retire *eventually* (in Sub 6 cutover); they spawn as normal until then.

- `MobType::Zombie`
- `MobType::Skeleton`
- `MobType::Creeper`
- `MobType::Spider`
- `MobType::Slime`
- `MobType::WitherSkeleton`
- `MobType::IronGolem` (replaced by `Knight`)

Their `MobType` variants stay declared in the enum forever (positional bincode pins them). After Sub 6, the spawn pool tables drop them; a one-shot save-load scrub despawns any still in existing worlds.

---

## New roster

| Category | Entries | Notes |
|---|---|---|
| **Human brigands** | Brigand, Marauder, Berserker | All live in Brigand Hideouts. Patrol/raid at night, return to hideout by dawn. Tier identity via HP / AI / drop scaling. |
| **Wild animals** | Bear (new), Hyena (new). Wolf stays. | Bear: food-interested, raids player food chests + crops. Hyena: Savanna pack-hunter. Wolf: already tameable + danger-if-attacked (PR #49). |
| **Village defenders** | Knight | Replaces Iron Golem. Same auto-spawn rule (village with ≥3 villagers + ≥5 houses). Sword-equipped melee patroller. |
| **Structures** | Brigand Hideout | Worldgen, away from villages (≥128 blocks), biome-spread. Houses brigands + grows a visible stolen-goods stockpile chest. |

---

## Rename table (Naming Pass, HP-0)

Items where Mojang's term is coined/quirky and the proper historical term differs. Mojang-coincidence renames (Furnace, Pickaxe, etc.) are NOT in this table — they stay because both we and Mojang independently picked the correct historical English.

### Display-name updates (Item::name / BlockDef::name only)

| Current | Historical name | Identifier change? |
|---|---|---|
| `MaterialId::Bonemeal` display | **Bone Meal** (two words) | No |
| `MaterialId::HoneyBottle` display | **Honey Jar** | No |
| `BlockId::HAY_BALE` display | **Hay Rick** | No |
| `BlockId::BONE_BLOCK` display | **Bone Cairn** | No |
| `BlockId::AMETHYST_BLOCK` display | **Amethyst Cluster** | No |
| `BlockId::SUGARCANE` display | **Sugar Cane** (two words) | No |
| `BlockId::VENDOR_BLOCK` display | **Market Stall** | No |
| `BlockId::PLAN_TILE` display | **Plan Scroll** | No |
| `BlockId::CONSTRUCTION_ANCHOR` display | **Foundation Stone** | No |
| `BlockId::ARCHITECT_PLAQUE` display | **Mason's Mark** | No |
| `BlockId::DRAFTING_TABLE` display | **Drafting Bench** | No |

### Identifier + display renames (deeper code-rename)

| Current | Historical name |
|---|---|
| `MobType::WanderingVillager` + `data/mobs/wandering_villager.toml` | **`MobType::Peddler`** + `data/mobs/peddler.toml` |
| `Profession::Librarian` | **`Profession::Scribe`** |

### Strip entirely (no historical anchor)

| Current | Plan |
|---|---|
| `MaterialId::GlowBerry` | Mark deprecated alongside the other source-retired materials (no current recipe uses it; bincode-positional so variant stays declared). |

### Spec-doc renames (forward — used in later subs)

| Current/proposed | Historical name |
|---|---|
| Pillager (proposed tier name) | **Brigand** |
| Bandit Camp (proposed structure) | **Brigand Hideout** |
| Raid (Spec 22 mechanic) | **Incursion** (display only; the `raid.rs` module identifier stays for code-history continuity) |
| BanditLeaderTrophy (proposed) | **`MaterialId::BrigandChieftainTrophy`** |

---

## Design principles

- **Historical accuracy wins.** Name things what a medieval person would name them. Mojang-coincidence is fine; Mojang-coinage gets replaced.
- **Kid-friendly visuals.** Puff-of-smoke on defeat, no blood, no gore, no corpses. All enemies die on HP=0; tier identity via HP / AI / drop scaling (Brigands low-HP and flee-prone, Berserkers high-HP and never flee). No special-case "morale" or "KO" plumbing — same mechanic, different tuning per tier.
- **Hideouts are antagonists, not scenery.** Each Brigand Hideout accumulates the stolen goods its inhabitants bring back into a visible stockpile chest. The player has an ongoing incentive to find and clear hideouts to recover what's been taken.
- **"Concrete, not cards" applies to retired content.** Retired `MaterialId` and `MobType` variants stay declared (positional bincode keeps the wire format stable) but are marked `// DEPRECATED 2026-05-22: historical pivot retired fantasy source.` No removal of variants at any point.
- **Selective fantasy.** Kept elements (Nostrich, Satori) stay because they're economically or narratively load-bearing. The bar for keeping a fantasy element: "removing it would cost more than building its replacement."
- **Migration is a single discrete event.** Subs 2-5 layer the new content alongside the old (both rosters coexist during the transition). Sub 6 is the cutover — single PR flips spawn pools off + scrubs existing worlds. Avoids partial-migration confusion in playtest.

---

## Seven-sub decomposition

| # | Sub | Foundation doc | Scope |
|---|---|---|---|
| **0** | **Naming Pass** | `docs/foundations/2026-05-22-historical-pivot-naming-pass.md` (specced) | Apply the rename table above. Mostly display-name updates; two identifier renames (Peddler, Scribe); strip GlowBerry. ~150 LOC mechanical work. **Lands FIRST.** |
| 1 | **Drop-Economy Migration** | `docs/foundations/2026-05-22-historical-pivot-drop-migration.md` (specced) | Replace recipe-critical drops (Bone source-add reserved for Sub 2; Wool → 4 String recipe ships now); deprecate dead drops (Gunpowder, SpiderEye, RottenFlesh, Slimeball); replace WitherSkull with `BrigandChieftainTrophy`. Lands SECOND. |
| 2 | **Wild Animals + Chest** | `docs/foundations/2026-05-22-historical-pivot-wild-animals.md` (DELIVERED 2026-05-22) | Bears (Forest/Taiga, food-interested, 12-block scan, satiety, eats crops + raids chests) + Hyenas (Savanna pack 2-4, day-Lazy/night-Hunt + PackBoost). Wolves audit (Bone drop test promise). New `bear_ai.rs` + `hyena_ai.rs`. Also ships `BlockId::CHEST = 112` + 27-slot UI + save/load (HP-3 inherits). PROTOCOL_VERSION → 25 for EntityKind::Bear/Hyena. Axolittle playtest pending. |
| 3 | **Brigand Hideouts + 3 Human Tiers** | (to write) | Biggest sub. Worldgen `BrigandHideout` structure with palisade + central campfire + visible stockpile chest. `Brigand`, `Marauder`, `Berserker` MobTypes with per-tier scaling. Hideout-economy logistics: brigands raid → stockpile grows → player assaults hideout to recover. |
| 4 | **Knights** | (to write) | Replaces Iron Golem. Same auto-spawn rule from Spec 19 (village with ≥3 villagers + ≥5 houses). Sword-equipped melee patroller; engages brigands + hostile animals. Spec 19's `IronGolem` spawn arm reroutes to `Knight`. |
| 5 | **Spec 22 Incursion Wave-Composition Refresh** | (to write) | Replace `WaveKind::{Small/Medium/Large}` fantasy mixes (Zombie/Skeleton/Spider/Creeper) with brigand mixes (Brigand + Marauder + Berserker). Incursion spawn origin shifts from village perimeter to nearest Brigand Hideout (within ~256 blocks). Knights defend (replacing Iron Golem's defensive role). |
| 6 | **Migration Cutover** | (to write) | Single PR. Spawn-pool tables drop retired MobTypes. Iron Golem auto-spawn reroutes to Knight. One-shot save-load scrub walks ECS entities, despawns any retired MobType. Items already in inventory (RottenFlesh, SpiderEye) NOT scrubbed — they sit unused. Doc updates: Spec 5 §3 + §4 marking the pivot complete. |
| **7** | **Miller / Baker / Brewer Professions** | (to write; parallelisable with Subs 2-5) | Three new `Profession` variants (Miller, Baker, Brewer). Workstation-claim arms: Mill → Miller, Oven → Baker, Aging Rack → Brewer. Adds vocational depth to villages — currently the Mill/Oven/AgingRack workstations exist but no profession claims them. ~200-300 LOC. Independent of other historical-pivot subs (touches `villager.rs` only). |

---

## Sequencing rules

- **Sub 0 must merge before Sub 1 builds.** The Naming Pass renames structures and types that the Drop Migration touches; doing them in the opposite order causes pointless re-renaming.
- **Sub 1 must merge before Subs 2-5 build.** Recipe-critical drops need their replacements available before any later spec relies on them.
- **Subs 2-4 are parallelisable** — different files, different mob types, no shared state.
- **Sub 5 builds after Sub 3** — needs brigand `MobType` variants live in code.
- **Sub 6 lands last.** Don't ship until Subs 1-5 are all on `main` and playtested at least once.
- **Sub 7 is parallelisable with everything from Sub 1 onward.** Touches only `villager.rs`. Can ship any time after Sub 0 (so the Mill/Oven/AgingRack display names are stable).

---

## Migration posture

- **Existing saves** — fantasy mobs already alive in worlds get scrubbed at Sub 6 load-time pass (one-shot ECS walk; mark for despawn-on-next-tick).
- **Items in player inventories** (RottenFlesh, SpiderEye, Slimeball, Gunpowder, WitherSkull, GlowBerry) — NOT scrubbed. They sit unused in inventory until the player drops them. Their `MaterialId` variants remain in the enum forever (positional bincode); their `Item::name` + `Item::color` entries stay too. Marked as deprecated in source.
- **New worlds (post-Sub 6)** — no fantasy mobs spawn. Period.
- **During Subs 2-5** — both rosters coexist. Players in playtest see Bears + Hyenas + Knights alongside Zombies + Skeletons + Iron Golems. This is deliberate: it lets Axolittle playtest each new piece in isolation against the familiar baseline without losing visibility into regressions.

---

## Cross-spec hooks

- **Spec 22 Raid Defence** — Sub 5 swaps wave composition. The Sub 5 spec will rewrite `raid.rs::wave_composition` to use brigand types. Treasury inflow + bounty payout + Charter gating are unchanged. (Module identifier `raid.rs` stays for code-history continuity; user-facing "Incursion" wording lives in display/gossip strings.)
- **Spec 19 Villages & Villagers** — Iron Golem auto-spawn arm reroutes to Knight (Sub 4). Reputation system, quest payouts, dialogue framework are all unchanged.
- **Spec 28d Mob Roster** — Bears + Hyenas + Knights register as new MobTypes following the existing per-species pattern (Subs 2 + 4). The retired fantasy MobType variants stay declared.
- **Spec 5 Gameplay Systems** — §3 (mobs) + §4 (combat) doc updates mark the historical pivot complete (Sub 6).
- **Player Guide** — refresh pages mentioning Zombies/Skeletons/etc. with brigand + animal equivalents (Sub 6 or follow-on).

---

## Memory-rule check

- ✓ uk english naming — "Brigand" / "Marauder" / "Berserker" + "armour" / "behaviour" / "Defence" throughout. UK English spellings for renames (Honey Jar, Hay Rick, Sugar Cane two-words per UK English convention).
- ✓ axenstax has farming — orthogonal; farming food chain is unaffected.
- ✓ bitcoin parent controlled — orthogonal; Charter gating on Spec 22 bounty stays. Brigands don't introduce new sats flows.
- ✓ shared infra strategy — Brigand Hideout structure + hideout-economy stockpile pattern lifts cross-game. Knight (sword-equipped melee patroller) is engine-generic.
- ✓ proof of play is proof of work — no compliance impact; the pivot is purely aesthetic.

---

## Out of scope for this vision doc

- **PvP brigands.** Player-led "go bandit" gameplay is the Conflict-economy spec, separate.
- **Hideout NPCs that aren't brigands.** No hostage rescues, no captive villagers; v2.
- **Faction wars between hideouts.** v2.
- **Late-medieval / early-modern weaponry** (firearms, gunpowder repurposing). Out of scope; could revisit if a future era expansion lands.
- **Brigand boss fights.** Sub 3 spec may add a per-hideout Chieftain; today's vision leaves it open.
- **Wolves as wild aggressors** (independent of taming) — already covered by existing `wolf.rs` AI; no change needed.
- **Renaming Furnace / Pickaxe / Crafting Table / etc.** — these are correct historical English; the Mojang-coincidence isn't infringement. Keep.
- **Cooper / Tanner / Fletcher / Mason professions.** Sub 7 ships Miller/Baker/Brewer (the three workstation-bound ones); these other historical guild trades are deferred to a possible Sub 8+ unless a future foundation needs them.

---

## Open design questions for later sub-specs

1. **Knight equipment ladder** — single-tier Iron sword always, or scale Knight HP/weapon to village reputation tier? (Sub 4)
2. **Brigand Hideout visibility** — show hideout icons on the map at all times, or only after the player has scouted within range? (Sub 3)
3. **Stockpile chest contents** — exact composition rules. Random sample of nearby-village goods? Weighted by what's been stolen? (Sub 3)
4. **Hyena Savanna density** — pack size, despawn radius, day-vs-night activity tuning. (Sub 2)
5. **Bear food-raid mechanic** — does a Bear actually break a chest open, or just sniff and leave if it can't enter? (Sub 2)
6. **Existing fantasy items in inventory after cutover** — keep as inert sentimental items, or `/give` cleanup command? (Sub 6)
7. **Skep vs Bee Hive** — should we go further on the bee-keeping naming (Skep is the medieval term for a woven straw bee dome)? Deferred; "Hive" is generic enough to keep without rename. (Sub 0 doesn't touch it.)
8. **Workbench vs Crafting Table** — "Workbench" is marginally more medieval. Deferred; both terms are historical, and "Crafting Table" reads more obviously in-game. (Sub 0 doesn't touch it.)

These get answered when each sub-spec is brainstormed.
