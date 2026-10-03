# Historical Pivot — Sub-Foundation 5: Incursion Wave-Composition Refresh

**Status:** READY TO BUILD (sequenced after HP-3). **Prerequisite:** HP-3 (Brigand Hideouts) — needs `MobType::Brigand` / `Marauder` / `Berserker` live in code. HP-4 (Knights) merged so the village defender side of the pivot is in.
**Branch:** `feat/historical-pivot-sub5-incursion-refresh` off `main`.
**Trigger:** Sub-foundation **5** of the [Historical Pivot Long-Run](../vision/historical-pivot-long-run.md). Re-aims Spec 22 raid mechanics from the fantasy roster (Zombie/Skeleton/Spider/Creeper) to brigand tiers. User-facing wording shifts toward "Incursion" but the `raid.rs` module name stays for code-history continuity per the vision doc.

---

## TL;DR

Two-line patch in `raid.rs::WaveKind::composition` — swap fantasy mob lists for brigand-tier mixes, keeping total mob counts (5 / 10 / 20) and the scheduler / kill-attribution / bounty pipeline unchanged. Scales the threat in a clean way:

- **Small** — 4 Brigand + 1 Marauder. Beginner-friendly; one elite, the rest grunts.
- **Medium** — 6 Brigand + 3 Marauder + 1 Berserker. First Berserker shows up.
- **Large** — 10 Brigand + 7 Marauder + 3 Berserker. Multiple Berserkers; serious push.

That's it on the engine side. The smoke-pillar red-shift warning, treasury drain, defender bounties, reputation effects, Charter gating, Vendor Block raid-supplies highlight, raid leaderboard, gossip rumours — all unchanged. Brigand mobs use the same chase / patrol AI they got in HP-3; the wave just spawns them out of the village perimeter (Spec 22's existing spawn-origin) rather than from any Brigand Hideout.

---

## Why this lives here

- **Closes the loop:** HP-3 introduced brigands as a worldgen threat; HP-5 makes them THE raid roster too. By the time HP-6 retires the fantasy mobs, raids already use the new content.
- **Tiny patch surface:** isolated to `WaveKind::composition` + its `assert_total_matches_table` tests. No new fields, no PROTOCOL bump, no save migration.
- **Validates the existing raid scaffolding** is mob-roster-agnostic — the cleanest "this design didn't lock us in" result we'll get out of Spec 22.

---

## What this PR ships

### 1. Updated `WaveKind::composition`

```rust
pub fn composition(self) -> &'static [(MobType, u32)] {
    match self {
        // Small: 4 Brigand + 1 Marauder. Lots of bodies, one elite.
        WaveKind::Small => &[
            (MobType::Brigand, 4),
            (MobType::Marauder, 1),
        ],
        // Medium: 6 Brigand + 3 Marauder + 1 Berserker. First Berserker.
        WaveKind::Medium => &[
            (MobType::Brigand, 6),
            (MobType::Marauder, 3),
            (MobType::Berserker, 1),
        ],
        // Large: 10 Brigand + 7 Marauder + 3 Berserker. Multi-Berserker
        // push; serious threat.
        WaveKind::Large => &[
            (MobType::Brigand, 10),
            (MobType::Marauder, 7),
            (MobType::Berserker, 3),
        ],
    }
}
```

Totals match `WaveKind::total_mobs` (5 / 10 / 20). The existing `total_mobs` const stays the source of truth; the per-tier counts in composition sum to the same number.

### 2. Wave mobs don't get HomeHideout tagged

Brigand mobs spawned by `tick_raid_spawn_wave` are wave-only — they're not affiliated with any Brigand Hideout. The kill-attribution path on `RaidMember` works the same as before for these; the `HomeHideout` ECS component that HP-3 added is **not** attached to raid mobs. Consequence: killing a Berserker that spawned as part of a Large raid drops the trophy (the kill-attribution is mob-type-keyed, not raid-context-keyed) — that's intentional, raises the value of defending a large incursion.

The brigand override pass (`brigand::tick_brigand_overrides`) still applies to wave-mobs because they're `MobType::Brigand`/`Marauder`/`Berserker`. The flee gate looks for `HomeHideout`, so wave-Brigands without one fall through to the "Idle" branch (the brigand pass stands them still rather than chasing the player to death). The intended behaviour for raids is "wave mobs press the village" — so we'll prevent the flee-but-no-home stall by **skipping the flee gate when no HomeHideout exists** (current code does this via the `if let Ok(home)` branch — the `else` branch sets `Idle { timer: 60 }`, which is functionally a stall).

To fix this cleanly: when a brigand-family mob has no `HomeHideout`, the flee gate falls through to the regular detect-range override (i.e. wave-Brigands behave like normal hostile mobs when wounded — they keep chasing rather than fleeing). One small edit in `brigand.rs::tick_brigand_overrides`.

### 3. Renamed gossip / dialogue strings

Where Spec 22's villager dialogue refers to "raid" in pre-raid gossip + heroes-here-roster strings, refresh the player-facing copy to use "Incursion" / "raiders" → "brigands" where it reads naturally. The `raid.rs` module identifier stays.

Search-and-replace scope is small (gossip pools live in `villager.rs`; raid-warning toasts in `game_loop.rs`).

### 4. (No PROTOCOL bump)

Wave composition is server-side data; mob discriminants already on the wire from HP-3. No EntityKind add, no save-shape change.

---

## What this PR does NOT do

- Doesn't change scheduler, treasury drain, bounty payout, reputation effects, Charter gating, smoke-pillar warning, leaderboard, gossip-during-warning, vendor highlight — all the existing Spec 22 + Phase 13-18 work stays as-is.
- Doesn't switch raid spawn origin to Brigand Hideouts (the vision doc mused about hideout-to-village raid feedback loops; that's v2).
- Doesn't add a new `RaidEvent` variant — same Spec 22 single-round MVP shape.
- Doesn't rename the `raid.rs` module. Code-history continuity > display naming.
- Doesn't tune the per-wave total counts. Same 5 / 10 / 20 totals.

---

## Phasing

| # | Phase | Files | LOC est |
|---|---|---|---|
| 1 | **This spec** | — | — |
| 2 | Update `WaveKind::composition` + the per-kind comments | `raid.rs` | ~50 |
| 3 | `brigand.rs` — let wave-mob flee gate fall through to detect-range when no HomeHideout is attached (wave Brigands don't have homes) | `brigand.rs` | ~30 |
| 4 | Refresh villager gossip + raid-warning toast copy ("raiders" → "brigands"; "Raid" → "Incursion" in dialogue, not in module identifiers) | `villager.rs`, `game_loop.rs` | ~50 |
| 5 | Update existing Spec 22 composition tests to match the new mob mixes | `raid.rs::tests` | ~80 |
| 6 | New tests: tier-content checks (each kind has expected mob mixes), totals still match `total_mobs`, no fantasy MobTypes in composition | `raid.rs::tests` | ~70 |
| 7 | Docs (HP-5 spec → DELIVERED, README flip, vision doc spec 22 callout) | docs | ~20 |
| 8 | Axolittle playtest | — | — |

**Total**: ~300 LOC. Tiny PR.

---

## Tests

```rust
#[test] fn small_wave_is_four_brigands_plus_one_marauder() {
    let comp = WaveKind::Small.composition();
    assert!(comp.iter().any(|(m, n)| *m == MobType::Brigand && *n == 4));
    assert!(comp.iter().any(|(m, n)| *m == MobType::Marauder && *n == 1));
    let total: u32 = comp.iter().map(|(_, n)| n).sum();
    assert_eq!(total, WaveKind::Small.total_mobs());
}

#[test] fn medium_wave_includes_first_berserker() {
    let comp = WaveKind::Medium.composition();
    assert!(comp.iter().any(|(m, n)| *m == MobType::Berserker && *n == 1));
    let total: u32 = comp.iter().map(|(_, n)| n).sum();
    assert_eq!(total, WaveKind::Medium.total_mobs());
}

#[test] fn large_wave_has_multiple_berserkers() {
    let comp = WaveKind::Large.composition();
    assert!(comp.iter().any(|(m, n)| *m == MobType::Berserker && *n >= 2));
    let total: u32 = comp.iter().map(|(_, n)| n).sum();
    assert_eq!(total, WaveKind::Large.total_mobs());
}

#[test] fn no_fantasy_mobs_in_any_wave_after_hp5() {
    for kind in [WaveKind::Small, WaveKind::Medium, WaveKind::Large] {
        for (m, _) in kind.composition() {
            assert!(matches!(
                m,
                MobType::Brigand | MobType::Marauder | MobType::Berserker
            ), "wave {kind:?} contains fantasy mob {m:?}");
        }
    }
}

#[test] fn wave_brigand_without_home_does_not_flee_to_nowhere() {
    // Spawn a wave-style Brigand (no HomeHideout component), set HP low.
    // The override pass should NOT stall it — it should treat the low-HP
    // brigand as a normal hostile (chase the player).
    // ...
}
```

---

## Acceptance

- `./check.sh` ALL GREEN.
- All Phase 5-6 tests pass.
- A test-world raid produces brigand mobs (Brigand/Marauder/Berserker), no fantasy mobs.
- Wave totals still match `WaveKind::total_mobs` (5 / 10 / 20).
- A wave-spawned Brigand at low HP keeps chasing (no `HomeHideout`-stall path).
- Phase 8 (playtest): Axolittle confirms the raid feels human now — brigands feel different to fight than the fantasy mobs did; Berserker-led pushes have weight.

---

## Memory-rule check

- ✓ uk english naming — UK English throughout ("Incursion", "behaviour", "defence").
- ✓ bitcoin parent controlled — orthogonal; bounty / treasury / Charter gating unchanged.
- ✓ shared infra strategy — the wave-composition table was already engine-generic; HP-5 just validates that by switching the roster without engine-wide ripple.
- ✓ merge to main preauthorised — healthy-gate merge when `check.sh` is green.

---

## Out of scope (deferred to later subs or v2)

- Raid spawn origin shifting from village perimeter to nearest Brigand Hideout — v2.
- Multi-round raids — Spec 22 explicitly deferred this; HP-5 doesn't change that.
- Per-tier raid composition tuning post-playtest — adjust the table once Axo's seen waves play out.
- Renaming `raid.rs` to `incursion.rs` — code-history continuity > display naming.

---

## Open questions

None. All design decisions resolved during HP-5 brainstorming (autonomous-mode call).
