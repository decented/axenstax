# The Workshop — blueprint authoring (build with normal blocks, stamp, lock in, tagged *creative*)

**Status:** ✍️ **DRAFTED 2026-06-08** — idea captured, **design NOT complete** (one open fork needs
its own brainstorm pass before build — see §Needs a brainstorm). No build (project rule).

**Sibling to** `2026-06-08-workshop-blow-up-and-sculpt.md`. The Workshop is a **design room** with
two focused tools:

- **Appearance tool — the Bellows** (the sibling spec): redesign how a block *looks* (reskin / sculpt).
- **Structure tool — the Blueprint** (this spec): capture a *structure* out of **normal blocks** —
  *build it, stamp it, lock it in.* **No blow-up, no shrink.**

Owner ask (2026-06-08): *"use the [Workshop] space for doing blueprints… put them on a blueprint,
lock them in… normal blocks on a blueprint… marked as creative, not survival."*

---

## TL;DR

You can already capture builds into blueprints (the **Drafting Stamp** + connectivity-flood capture,
shipped — `docs/foundations/2026-06-04-blueprint-column-capture.md`). This doc makes the **Workshop**
a first-class place to do it: build a structure out of normal blocks on the Workshop floor, stamp it,
and it locks into a saved blueprint — **tagged as authored in creative**, so it never masquerades as a
survival-earned build. It's **mostly reuse**; the genuinely-new bits are small: (1) confirm capture
works inside the Workshop world, (2) stamp the **creative provenance** flag, and (3) decide what that
flag *means* when the blueprint is later used in survival (the one real open question).

---

## Context pointers (build-state, audited 2026-06-08)

**Already BUILT (reused — no new capture tech):**
- **Capture + Drafting Stamp.** `plan::capture` / `capture_connected_volume` / `flood_fill_volume`;
  the reusable **Drafting Stamp** trigger (`blueprint_attach.rs:87`) — *build-on-paper* + stamp,
  shipped 2026-06-04 (`project_blueprint_column_capture_shipped`).
- **Plan provenance already exists.** `PlanData.authored_in: String` (`plan.rs:200-201`, **defaults
  `"survival"`**, `:219`); `develop_state` + `develop_state_color` (`plan.rs:114`); `PlanKind`. The
  Workshop reshape path already stamps `authored_in: "workshop"` (`workshop::capture_box_as_plan`).
  So the **provenance hook is in place** — this doc just sets + honours it for blueprints.
- **The Workshop world.** A saved creative world (`is_workshop`, `WORKSHOP_FOLDER`) — capture is
  world-agnostic, so the Stamp should already function there; this is a *confirm + wire*, not a build.
- **Integrity ledger.** `WorldMeta` `ever_creative` / `pure_survival` / `cheats_used` — the existing
  creative-vs-survival honesty surface a Workshop blueprint's `creative` tag aligns with.

Related: `2026-05-27-blueprint-cyanotype.md`, `2026-06-04-blueprint-face-attachment.md`,
`plan_registry.rs`, `plan_ui.rs`.

---

## The gesture

1. **Build** a structure on the Workshop floor with **normal blocks** (creative inventory, no blow-up).
2. **Stamp** it (Drafting Stamp) → connectivity-flood capture → a `PlanData`.
3. **Lock it in** → saved to your plan registry, **`authored_in` = workshop/creative**.

The Workshop is the natural home: it's already a clean creative space to lay a structure out and
stamp it, separate from your survival worlds.

## Provenance: "creative, not survival"

A blueprint built in the Workshop is **honestly tagged creative** (you had infinite blocks, flight,
no survival cost). This is consistent with the existing ledger (`ever_creative` / `pure_survival`):
the blueprint records *where it was made*, so it can be used and shared but never **claimed as a
survival achievement**. Display can surface the tag (mirror `develop_state_color`).

---

## Needs a brainstorm (the one real open fork — resolve before build)

**What does the `creative` tag *do* downstream?** Building from a Workshop (creative) blueprint in a
**survival** world is the question — pick the policy with the owner:

- **A — provenance only (lightest).** The tag is informational; the build still costs survival
  materials as normal. Honest labelling, no mechanical gate. *(Likely best — preserves survival
  economy; the tag is just truth-in-advertising.)*
- **B — taints the result.** A structure placed from a creative blueprint carries the creative mark
  (the build isn't survival-pure), but is otherwise free-form.
- **C — survival-locked.** Creative-authored blueprints can't be *placed* in a pure-survival world at
  all (only re-built by hand). Strongest, probably too strict.

This fork is **not** decided — it's a gameplay/economy call (Axolittle + owner), and it interacts
with Proof-of-Play / survival purity. **Brainstorm before building.**

---

## Phased scope (after the fork is decided)

- **Phase 1 — capture in the Workshop.** Confirm/wire the Drafting Stamp inside the Workshop world;
  stamp `authored_in = workshop`. **Acceptance:** stamping a Workshop build yields a `PlanData`
  tagged workshop; round-trips serde; `check.sh` green.
- **Phase 2 — the creative tag + its policy.** Implement the chosen downstream policy (§fork) +
  surface the tag in the plan UI. **Acceptance:** per the chosen policy; `check.sh` green.

---

## Out of scope / deferred

- **New capture tech.** Reuses the shipped Stamp + flood-fill — nothing re-derived.
- **Appearance redesign.** That's the Bellows (sibling spec).
- **Sharing of Workshop blueprints.** Plans already have a sharing story; the *creative tag* in a
  shared plan is a small addition, designed when wanted.

---

## Memory-rule check

- **`project_shared_infra_strategy`:** the Workshop-as-design-room + provenance-tagging are
  engine-generic; the blueprint system is already shared infra. ✅
- **`reference_proof_of_play_is_proof_of_work`:** the creative-vs-survival honesty tag is *exactly*
  the integrity surface that keeps creative-made content from masquerading as proof-of-play work —
  the downstream policy fork must respect it. ✅
- **`feedback_autonomy_to_playtest_boundary`:** Phase 1 (capture + tag) is solo-buildable once the
  fork is picked; the policy + feel are owner/Axolittle calls. ✅
- **`feedback_uk_english_naming`:** UK English throughout. ✅
- **No build yet:** design/spec only; the downstream-policy fork needs a brainstorm before any code. ✅
