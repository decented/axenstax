# Combat feel — sweep attack + swing-arc hit detection

**Status**: ✅ BUILT 2026-06-16 (worktree `worktree-community-features`, goal `2026-06-16-community-features-solo-buildout`). Phase 1 feature **#23**.
**Date**: 2026-06-16
**Backlog**: `docs/research/2026-06-15-native-bake-in-feature-backlog.md` §23.

---

## TL;DR

A melee swing now **sweeps**: it hits the primary (crosshair) mob for full damage, and **every other mob inside the swing arc** for reduced sweep damage + a light knockback — the Minecraft-style sweep that makes fighting a crowd feel right. The hit detection is a shared, pure swing-arc predicate (reach + forward cone).

## Design (concrete, not cards)

- **`combat::in_swing_arc(to_target, look_dir, reach, min_dot)`** — pure geometry: a target counts if it's within `reach` **and** inside the forward cone (`to_target·look_dir ≥ min_dot`, ≈ 60°). Shared by the primary-target pick and the sweep. Unit-tested (front/far/behind/side).
- **`player_attack`** — after the primary hit lands, collects every *other* entity in the arc (immutable query → ids), then applies `SWEEP_DAMAGE_FRACTION` (0.4) × damage + half-knockback to each, stamping `LastAttacker` so kill-credit is correct. Two integration tests (a second mob in the arc is also hit; a mob behind is spared).
- Constants: `SWEEP_DAMAGE_FRACTION = 0.4`, `SWING_MIN_DOT = 0.5` (the existing cone), reusing `ATTACK_REACH = 3.0`.

## Solo boundary → playtest gate

Solo: the swing-arc geometry + the sweep damage/credit are unit-tested headless. **Playtest** (owner, needs a display + mobs): the *feel* — sweep damage fraction, arc width, knockback, and the **swing animation** (a viewmodel arc on attack, deferred below — purely visual). **PvP balance is multiplayer-gated** (this is the vs-mob model only, per the plan).

## Deferred (named)

- **Swing animation** — the held-item/arm viewmodel arc on attack (visual polish; feel = playtest).
- **Swept *volume* vs cone** — a true swept capsule/AABB instead of the reach+cone approximation, if the cone reads as too forgiving/strict.
- **Sweep particles / sound**, sweeping-edge enchant analogue, and **PvP** tuning (when multiplayer lands).

## Spec maintenance

Spec 05 (Gameplay) §6 combat note references the sweep + `in_swing_arc`.
