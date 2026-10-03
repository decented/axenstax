# Satoshi the Cloaked Founder-Sage — mystical-character design

**Make Satoshi a mystical, cloaked sage — an Obi-Wan-in-*A New Hope* hermit — across three layers: a hooded cloak, a wise-but-warm voice, and a quietly magical presence. Reuse-heavy, contained, kid-safe.**

- **Date:** 2026-06-25
- **Status:** ✅ **P0–P3 DELIVERED** (verified in code 2026-09-06 during the build-status audit: `entity_model.rs` Satoshi cloaked model + `satoshi_emissive_part` amulet glow via the emissive vertex sentinel, `satoshi.rs::build_house` interior torch, sage-voice corpus, `starter_hut_plan` gift). **P4 materialise fade-in NOT built** — optional, playtest-gated; the only remaining piece. Originally: READY TO BUILD (design approved by owner 2026-06-25; glow = **Option B / fullbright** chosen).
- **Posture:** Concrete, reuse-first. Builds on the existing Satoshi (`satoshi.rs`) + the villager entity-model path. The only genuinely new code is one model function, one `ModelPart` flag, a few textures, and a dialogue rewrite.
- **Companions:** `2026-06-22-satoshi-onboarding-foundation-spec.md` (the onboarding character this re-skins), `2026-06-23-satoshi-onboarding-v2-design.md`, `../../vision/genesis-founding-myth-long-run.md` (Satoshi = the founder figure; a mystical founding-sage *is* the moonshot's Satoshi).

---

## 0. Guardrails (ABSOLUTE — carried from the onboarding design)

These do not change and gate every phase:

1. **Warm, never scary.** Obi-Wan-mysterious, *not* creepy-hooded-stranger. Hood up, but the **face stays softly visible and kindly**. A mysterious *helper*, never a threat. (Review gate: a reviewer confirms the look reads as warm to a child.)
2. **Still fully scripted, no money/earning words.** The voice rewrite stays in the authored corpus scanned by `corpus_has_no_money_or_earning_words` — every new line must pass it (`sat`, `bitcoin`, `earn`, `payout`, `wallet`, `money`, `reward`, whole-word). No live AI, no network, no free-text.
3. **Minimal reading.** Sage ≠ verbose. Lines stay short (the onboarding rule). Wise *tone*, not a monologue.
4. **Pull-first / anti-Navi preserved.** "Mysterious presence" must NOT make him initiate, chase, nag, or seize input. He still lives findably in his hut; you come to him; `should_initiate` stays pull-only. Mystery = *atmosphere*, not pestering.
5. **No over-claiming safety / UK English** — unchanged.

---

## 1. Scope (owner-confirmed: full treatment, Option B glow)

Three layers, each independently shippable + verifiable:

| Layer | What | Cost |
|---|---|---|
| **Look** | Hooded cloak/robe on Satoshi's skeleton, Obi-Wan earthy palette, hood up + face visible | small |
| **Voice** | Rewrite his corpus into a warm-sage register | trivial |
| **Presence** | (a) invitingly lit hut, (b) **self-glow via fullbright** (Option B), (c) gentle materialise fade-in | small–medium |

**Deferred (out of scope, honestly flagged):** floating sparkle/motes (needs a particle system — *not built*), light *radiating into the world* (needs new entity→lighting work), a staff (Obi-Wan ANH has none; add later if wanted), Workshop reskin of Satoshi.

---

## 2. Approach (reuse-first, verified in code)

- **Distinct model keyed on the marker.** `build_entity_model_vertices` (`entity_model.rs:844`) already queries the ECS *with the entity id available* and calls `mob_model(kind.0)`. Add `Option<&satoshi::SatoshiMarker>` to that query tuple and branch: `let model = if is_satoshi { satoshi_model() } else { mob_model(kind.0) };`. No new render system — Satoshi is still a `MobType::Villager`, just drawn from his own parts. (The **Peddler** is the proof this works: `entity_model.rs:2270` is "same skeleton as Villager with **purple hood/robe**" — we copy that pattern in Satoshi's palette.)
- **Self-glow with zero shader change.** `Vertex` already carries a `light` field and a `Vertex::FULL_BRIGHT` constant (used by projectiles, `entity_model.rs:2051`). Add a per-part `emissive: bool` to `ModelPart`; in the part-vertex builder, when `emissive`, write `Vertex::FULL_BRIGHT` into those verts' `light`. The shader already consumes `light` → the hood-edge/trim glow at full brightness regardless of ambient. **No shader/pipeline edit.**
- **Lit hut = reuse.** `build_house` (`satoshi.rs:129`) already plants a roof `TORCH`. Add one interior light block (torch/lantern) so the hut glows invitingly from within.
- **Materialise fade = reuse.** The avatar already supports a fade baked into vertex `light` (`entity_model.rs:1156`, the third-person screen-door dither). The fade-in reuses that path — a short alpha ramp when Satoshi (re)appears — rather than new tech.

This keeps everything **concrete** (CLAUDE.md): no bridges, no parallel systems, each layer testable in isolation.

---

## 3. Phases (ordered, each compiles + tests green + is mergeable)

### P0 — The cloaked model (solo-verifiable)
- New `satoshi_model() -> Vec<ModelPart>` in `entity_model.rs`: villager humanoid skeleton + **hood** (over/around the head, face front-face left visible), **robe body** (replaces the plain torso faces), **cloak back-drape** (a thin part behind the body), Obi-Wan **earthy palette** (tan/brown robe, darker hood). Slightly weightier silhouette than a plain villager; proportions otherwise unchanged.
- Branch model selection on `SatoshiMarker` in `build_entity_model_vertices` (add `Option<&SatoshiMarker>` to the query; do not key on a new `MobType`). Reskin (`overrides.mob_part_faces`) is **skipped for Satoshi** (he uses his own faces; Workshop-reskinning Satoshi is out of scope).
- New textures (procedural in `texture_gen.rs`, following the Peddler robe/hood pattern): robe, hood, cloak-drape, and a **luminous trim** accent layer (pale/indigo, used by P1).
- **Verify (solo):** Satoshi renders a model whose part-set differs from `villager_model()` (unit test on part count / a hood part present); the existing "exactly one Satoshi" test still holds; manual run shows a hooded, robed, face-visible Satoshi in his hut.

### P1 — The self-glow (Option B, solo-verifiable)
- Add `pub emissive: bool` to `ModelPart` (default `false`; all existing models unaffected — add `..Default`-style or set explicitly).
- In the part-vertex builder (`build_part_vertices`), when `part.emissive`, set the emitted verts' `light = Vertex::FULL_BRIGHT`.
- Mark **only** Satoshi's hood-edge / robe-trim (the luminous-trim faces) `emissive: true` — a faint, tasteful glow, not a lightbulb. The bulk of the robe stays normally lit.
- **Verify (solo):** a unit test that an `emissive` part's vertices carry `FULL_BRIGHT` while a normal part's don't, at a dark ambient; manual run in a dim hut shows his trim quietly glowing. **Playtest-tunable:** glow intensity / which parts glow.

### P2 — The invitingly lit hut (solo-verifiable)
- In `build_house`, add an interior light source (a wall `TORCH` or lantern block) alongside the existing roof torch, so the hut reads warm and "someone wise lives here" from the outside.
- **Verify (solo):** the lit block is placed (assert in the hut-build test); manual run shows the hut glowing.

### P3 — The sage voice (solo-verifiable)
- Rewrite the `SATOSHI_*` constants (`satoshi.rs:296–330`) into a warm-sage register. Draft set (final wording tunable):
  - `SATOSHI_GREETING` → *"Ah — there you are. I had a feeling the wind would bring someone today. You look weary; let me see to that."*
  - `SATOSHI_GIFT_LINE` → *"Take this. A small kindness for the road ahead."*
  - `SATOSHI_SCHEMATIC_LINE` → *"And this — a plan, old as these parts. Lay it down and build along; what it asks for, you'll gather as you go."*
  - `SATOSHI_RETURN_LINE` → *"Back again? Good. My door is always open to those who seek it."*
  - `SATOSHI_CREATIVE_GREETING` → *"Ah — a fresh world, and a maker's hands. Shall we begin?"*
  - `SATOSHI_MISSIONS_DONE` → keep the warmth; sage-ify lightly.
- **Verify (solo):** `corpus_has_no_money_or_earning_words` still passes (the load-bearing gate); lines stay short (a length-guard test is optional). **Playtest-gated:** does the voice land as *wise + warm* to a kid, not stuffy?

### P4 — Materialise fade-in (OPTIONAL polish; the playtest piece)
- When Satoshi (re)appears — on summon (N) arrival, or as the player approaches his hut — ramp his vertex `light`-baked alpha from faint→full over ~0.5 s (reusing the avatar dither-fade path) so he "settles into being" rather than hard-popping. A one-shot, transient (not persisted) effect; **never** seizes input; the player can walk away mid-fade.
- **Verify (solo):** the fade state advances and clears; movement is provably unaffected. **Playtest-gated (THE charm question):** does the appearance feel *magical*, not janky? Tune from the test, like the onboarding co-presence demo.

---

## 4. Hook points / file touch-list (real names)

| File | Change |
|---|---|
| `entity_model.rs` | New `satoshi_model()`; add `emissive: bool` to `ModelPart`; honour it in `build_part_vertices` (set `Vertex::FULL_BRIGHT`); branch model selection on `Option<&SatoshiMarker>` in `build_entity_model_vertices`. |
| `texture_gen.rs` | New procedural layers: Satoshi robe / hood / cloak-drape / luminous-trim (bump `texture_count`). Follow the Peddler robe/hood generators. |
| `satoshi.rs` | Rewrite the `SATOSHI_*` corpus (P3); add an interior light block in `build_house` (P2). Corpus stays scanned by the compliance test. |
| `satoshi.rs` / render glue | (P4, optional) a transient fade-progress field + a `tick`-driven ramp feeding the vertex alpha; reuse the avatar-fade path. |
| tests | Distinct-model test; emissive-vertex test; lit-hut assertion; compliance test (already exists — must still pass with new lines). |

**Not touched:** `MobType` enum (no new variant — keeps bincode/`mob_def` stable); the shader/pipeline (P1 uses the existing `Vertex.light`); `mob_model` for any other mob; save format (no new persisted field — model + glow are derived from the marker; the optional fade is transient).

---

## 5. Acceptance criteria

**Solo-verifiable (gate every phase):**
- [ ] `./check.sh` green (run with `CARGO_INCREMENTAL=0` for the known flaky linker).
- [ ] `corpus_has_no_money_or_earning_words` **passes** with the new sage lines.
- [ ] Satoshi renders his own hooded/robed model (distinct from `villager_model()`), face visible; "exactly one Satoshi" still holds.
- [ ] `emissive` parts carry `FULL_BRIGHT`; normal parts don't (unit test); his trim glows in a dim hut (manual).
- [ ] The hut has an interior light (assert + manual).
- [ ] No new `MobType`, no save-format change, no shader edit.

**Playtest-gated (needs a voluntary kid — the charm questions):**
- [ ] Does the cloaked Satoshi read as **warm-mystical, not creepy**? (The one thing the build can't self-validate — tune palette/hood/face + glow from the test.)
- [ ] Does the **sage voice** feel wise-and-warm, not stuffy?
- [ ] (P4) Does the **materialise fade** feel magical, not janky?

---

## 6. What we reuse vs. what's new

**Reuse:** the villager skeleton + Peddler hood/robe pattern · `build_entity_model_vertices` (already exposes the entity id) · `Vertex.light` + `Vertex::FULL_BRIGHT` (no shader change) · `build_house`'s torch lighting · the avatar dither-fade path (P4) · the whole onboarding state machine, persistence, and compliance test (unchanged).

**New (small):** `satoshi_model()` · `ModelPart.emissive` + its one-line honouring · ~4 textures · the rewritten corpus · (optional) a transient fade ramp.

---

## 7. Deferred / do-not-build

- **Particle aura / floating motes** — needs a particle system (not built). Big lift; defer.
- **Light radiating into the world** from Satoshi — needs entity→block-light work; the lit hut covers "he lights his space" cheaply. Defer.
- **A staff** — not in Obi-Wan ANH; optional future flourish.
- **Workshop reskin of Satoshi** — out of scope (he uses fixed faces).
- **AI-driven Satoshi** — remains do-not-build (safeguarding), per the onboarding design.
