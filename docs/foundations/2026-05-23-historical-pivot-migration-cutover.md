# Historical Pivot — Sub-Foundation 6: Migration Cutover

**Status:** READY TO BUILD (sequenced after HP-3 / HP-4 / HP-5). Lands LAST in the historical-pivot sequence.
**Branch:** `feat/historical-pivot-sub6-migration-cutover` off `main`.

> **SUPERSEDED IN PART (2026-05-24).** HP-6's decision to **keep the retired
> `MobType` / `EntityKind` / `drops_for` / `Item::name` variants declared forever**
> for positional-bincode wire stability was **reversed** by the
> [Fantasy-Roster Excision](2026-05-24-fantasy-roster-excision.md). That pass
> **removed the 7 fantasy mob variants and 5 fantasy drop items from the engine
> entirely** for open-source IP cleanliness, accepting the breaking wire/save
> change pre-launch (`PROTOCOL_VERSION` bumped to **39**; bones re-sourced from
> livestock to preserve the Bone→Bonemeal loop). Everything HP-6 says below about
> variants "staying declared forever" / "never removed" is **no longer current** —
> only the spawn-pool swap and the Iron-Golem→Knight defender change survive. See
> the excision foundation for the live state.
**Trigger:** Sub-foundation **6** of the [Historical Pivot Long-Run](../vision/historical-pivot-long-run.md). The single discrete event that retires the fantasy mob roster. New worlds post-cutover never see Zombies / Skeletons / etc.; in-session existing fantasy mobs disappear naturally when entities turn over (the alpha entity layer doesn't persist mobs across save / load, so no migration scrub is needed).

---

## TL;DR

A focused PR that flips the night spawn pool from fantasy mobs to brigands + animals, and removes the Iron Golem auto-spawn (the Knight from HP-4 is the sole village defender now). Retired `MobType` variants stay declared in the enum forever (positional bincode pins them); their `drops_for` arms stay too. Inventory items the player carries (RottenFlesh, SpiderEye, Slimeball, Gunpowder, WitherSkull, GlowBerry) remain inert; the player can drop them.

After this PR ships:
- A fresh world generates with no Zombies/Skeletons/Creepers/Spiders/Slimes/WitherSkeletons/IronGolems.
- Nighttime spawning produces Brigand (mostly) + Marauder (occasional) at the dark surface positions, with Bear / Hyena keeping their biome-gated arms.
- Brigand Hideouts (HP-3) continue producing Brigand/Marauder/Berserker via `tick_hideout_spawning`.
- Villages no longer auto-spawn Iron Golems; Knights (HP-4) are the lone defender.

---

## Why this lives here

- **Single discrete event.** The vision doc commits to one cutover PR rather than a slow drain, so the playtest after this ships shows a world that's fully historical-pivot. No "did Axo see a Zombie one last time?" weirdness.
- **Tight surface.** The actual code change is localised to `spawning.rs::tick_mob_spawning` + one game_loop call removal. The narrative weight is bigger than the diff.
- **Cross-game alignment.** Once this lands, the historical-pivot design contract is fulfilled — sibling games (other games on the same primitives) can adopt the same pattern without inheriting the alpha's fantasy roster.

---

## What this PR ships

### 1. Spawn-pool refresh in `spawning::tick_mob_spawning`

Current night-spawn branch picks a fantasy mob via `tag % 8`. Refresh swaps the fantasy arms for brigand-family arms:

```rust
let kind = match tag {
    0 | 1 | 2 | 3 | 5 => MobType::Brigand,            // ~62.5 %
    4 => MobType::Marauder,                            // ~12.5 %
    6 if is_forest_like || is_taiga_like => MobType::Bear,
    6 => MobType::Brigand,
    7 if is_savanna_like => MobType::Hyena,
    7 => MobType::Brigand,
    _ => MobType::Brigand,
};
```

Berserker remains exclusive to Brigand Hideouts — the boss tier is something the player has to hunt out, not stumble into in the dark.

Sun-burn stays on Zombie/Skeleton in the code arm for legacy / `/give`-created mobs that might still exist; the live spawn path no longer produces them so it's a dormant code arm.

### 2. Remove Iron Golem auto-spawn

Drop the `village_gen::tick_iron_golem_spawn(...)` call from `game_loop.rs`. The function itself stays in `village_gen.rs` with an `#[allow(dead_code)]` guard so it survives review without warnings — keeps the option to revive in a "Knight + Golem coexistence" branch if a future spec wants the choice. Knight auto-spawn (`tick_knight_spawn`) stays as-is.

### 3. No save-format migration

The alpha entity layer doesn't persist mobs across save / load (saves carry blocks + block-entities + world state, not the ECS). A pre-cutover world that was mid-session with Zombies in the air loses them naturally on the next session start; no scrub needed. This removed an entire dimension of risk from the cutover.

### 4. Documentation polish

- Spec 5 §3 (mobs) gets a "Historical pivot complete" callout citing this PR.
- The vision doc's "Retired fantasy" section gets a "DELIVERED" timestamp.
- `spawning.rs` doc comment refreshes to mention the new brigand-led night pool.

### 5. No PROTOCOL bump

No EntityKind changes, no save shape changes, no new BlockId. PROTOCOL_VERSION stays at 27 (HP-4's bump).

---

## What this PR does NOT do

- Doesn't remove the retired `MobType` variants — they stay declared forever (positional bincode wire stability + ergonomic test references).
- Doesn't remove the retired `drops_for` arms or `EntityKind` discriminants. Same wire-stability reason.
- Doesn't scrub player inventories of `RottenFlesh` / `SpiderEye` / `Slimeball` / `Gunpowder` / `WitherSkull` / `GlowBerry`. Players keep them as inert curios; `Item::is_food()` already excludes the rotten variants (HP-1 work).
- Doesn't add a `/cleanup-legacy-items` command. Players who want to be rid of them can drop them; deferred to v2 if it ever matters.
- Doesn't change Spec 22 wave composition (HP-5 already did that).
- Doesn't rename `raid.rs` — module identifier stays for code-history continuity.
- Doesn't add a one-time "Historical Pivot Complete" world toast. Quieter is better; the player notices because the mobs have changed.

---

## Phasing

| # | Phase | Files | LOC est |
|---|---|---|---|
| 1 | **This spec** | — | — |
| 2 | Refresh `tick_mob_spawning` night-spawn match arms | `spawning.rs` | ~40 |
| 3 | Remove `tick_iron_golem_spawn` call from game_loop; add `#[allow(dead_code)]` to the function | `game_loop.rs`, `village_gen.rs` | ~10 |
| 4 | Update existing spawn tests to match new pool (Brigand-dominant, no Zombie/Skeleton) | `spawning.rs::tests` | ~80 |
| 5 | Docs: spec → DELIVERED, README flip, vision-doc "Retired fantasy" callout, Spec 5 §3 callout | docs | ~30 |
| 6 | Axolittle playtest | — | — |

**Total**: ~160 LOC. Tiny PR — the actual code change is one match-block.

---

## Tests

```rust
#[test] fn night_spawn_yields_brigands_not_zombies() {
    // Across many ticks the deterministic seed should produce brigands;
    // never a Zombie/Skeleton/Spider/Creeper/Slime/WitherSkeleton.
    let world = fixture_floor(10);
    let mut ecs = hecs::World::new();
    for t in 0..200 {
        tick_mob_spawning(&mut ecs, &world, NIGHT + t, &[Vec3::new(0.0, 12.0, 0.0)]);
    }
    let mut saw_brigand = false;
    for (_, kind) in ecs.query::<&MobKind>().iter() {
        match kind.0 {
            MobType::Brigand | MobType::Marauder | MobType::Bear | MobType::Hyena => {
                saw_brigand = saw_brigand || matches!(kind.0, MobType::Brigand);
            }
            other => panic!("HP-6 night spawn produced retired mob {:?}", other),
        }
    }
    assert!(saw_brigand, "expected at least one Brigand across 200 spawn ticks");
}

#[test] fn night_spawn_never_produces_fantasy_mob() {
    // Stronger: zero fantasy mobs of any kind, ever.
    let world = fixture_floor(10);
    let mut ecs = hecs::World::new();
    for t in 0..500 {
        tick_mob_spawning(&mut ecs, &world, NIGHT + t, &[Vec3::new(0.0, 12.0, 0.0)]);
    }
    for (_, kind) in ecs.query::<&MobKind>().iter() {
        assert!(!matches!(
            kind.0,
            MobType::Zombie | MobType::Skeleton | MobType::Spider |
            MobType::Creeper | MobType::Slime | MobType::WitherSkeleton
        ), "found {:?}", kind.0);
    }
}

#[test] fn night_spawn_can_still_produce_bears_in_forest() {
    // Bear / Hyena biome-gated arms still fire.
    // Mock surface = GRASS (forest-like) at the spawn position.
    // ...
}
```

---

## Acceptance

- `./check.sh` ALL GREEN.
- All Phase 4 tests pass.
- New tests: night-spawn produces brigand-family + Bear/Hyena only; never produces a retired fantasy mob.
- A fresh world after this PR has no Iron Golems near the village (Knights spawn instead).
- Players who had legacy items (RottenFlesh, etc.) in inventory before the PR still have them after.
- Phase 6 (playtest): Axolittle confirms the world feels historical now — no fantasy mob has appeared.

---

## Memory-rule check

- ✓ uk english naming — UK English throughout.
- ✓ bitcoin parent controlled — orthogonal; sats payouts unchanged.
- ✓ shared infra strategy — the spawn-pool branch is generic; the historical-pivot pattern (retire roster A, swap to roster B, keep enum stable) is engine-generic.
- ✓ merge to main preauthorised — healthy-gate merge when `check.sh` is green.
- ✓ proof of play is proof of work — orthogonal.

---

## Out of scope (deferred to v2 or never)

- Removing retired `MobType` / `EntityKind` discriminants — never.
- Removing retired `drops_for` arms or `Item::name` entries — never (positional wire stability).
- Scrubbing legacy items from player inventories — v2 if ever needed.
- A "Historical Pivot Complete" toast or commemorative item — quieter is better.
- Renaming `raid.rs` to `incursion.rs` — code-history continuity.
- Removing `tick_iron_golem_spawn` entirely — kept under `#[allow(dead_code)]` for revival potential.

---

## Open questions

None. All design decisions resolved during HP-6 brainstorming (autonomous-mode call).
