# Moonshot Phase A — Theme Capture (Build Spec)

**The cheapest layer of the founding myth: frame the existing game as the Diamond Age awaiting its founding, and reframe the Genesis Block as a *world event* — using only copy, framing, and light touches over assets that already exist. No new core systems. The fun floor is untouched.**

- **Date:** 2026-06-22
- **Status:** 📋 **READY TO BUILD (MVP subset)** — design + task spec. The MVP is pure copy/flavour over existing assets and is solo-verifiable. Everything that touches a *mechanic* is explicitly DEFERRED and playtest-gated below.
- **Author:** Captured from the moonshot phasing in `docs/vision/genesis-founding-myth-long-run.md` §8.
- **North-star:** `docs/vision/genesis-founding-myth-long-run.md` (esp. §1 "not a Bitcoin game", §2 three-act premise, §6 fun-floor/meaning-ceiling, §7 knots, §8 Phase A, §9 guardrails).
- **Companions:** `docs/superpowers/specs/2026-06-22-guided-onboarding-satoshi-design.md` (the Satoshi guide is the same charming character that seeds this lore), `docs/vision/economies-long-run.md`.

---

## 1. Goal & principle

**Capture the *theme*, not the systems.** The north-star (§8) calls Phase A "~80% of the *meaning* for ~20% of the build." We do that by **wrapping the game that already exists in the founding-myth frame**:

- The world is the **Diamond Age** — a frontier that runs on the old money (diamonds), waiting for its founding.
- The **Genesis Block** (already in-engine, already singular-per-world, already fired on the first Satori) is reframed from "rare gem celebration" into **the founding event of your world's story** — *"your world's story begins."*
- A handful of **light lore touches** — loading tips, one world-creation flavour line, optional villager/vendor flavour — seed the Diamond-Age-awaiting-its-founding setting **diegetically**, with no lecture.

**The governing constraints (carried verbatim from the north-star §1, §6, §9 and the compliance memory):**

- **NEVER say "Bitcoin" in-world.** Money is *just money*: the old money is **diamonds**; sound money simply *becomes* the money over the (future) arc. No "Bitcoin," no "crypto," no "mining sats," no "earn."
- **Show, don't preach.** The setting carries the values. **No economics lecture**, no sermon, no "this is how money works" explainer in-world. (The meta-layer for the curious lives in docs/marketing, never in the play surface.)
- **No earning hooks. Payouts are deferred regardless.** The reward at the (future) founding is **legacy / founding standing**, never sats. Phase A introduces *zero* reward copy.
- **The fun floor stays exactly as-is.** Mine / build / farm / defend / explore is unchanged. **Theme enriches; it never gates.** If a touch changes *behaviour* (not just text), it is not in this MVP.
- **UK English** throughout (flavour, "behaviour", "colour", "favour", etc.).

**The one-line test for any Phase A change:** *Does it change only words/flavour shown to the player, leaving every mechanic, drop, save format, and behaviour byte-identical?* If yes → MVP-eligible. If no → DEFERRED (§ below).

---

## 2. Existing assets verified (the 20% we build *on*)

Every Phase A touch reframes something already in the engine. Verified against live code (`game/engine/src/`):

| Asset | Where it lives | What exists today | Phase-A reframe |
|---|---|---|---|
| **Genesis Block** | `save.rs` `WorldMeta.genesis_block_found: bool` (save.rs:2344) + `genesis_found_at_tick: Option<u64>` (save.rs:2390); claimed in `game_loop.rs::maybe_claim_genesis_block` (game_loop.rs:1127); **live celebration** at game_loop.rs:7164–7176 — `audio.play_genesis_block()` + toast `"Genesis Block! The first Satori of this world!"` (game_loop.rs:7167, 8 s). Scenario-mode end-card `"Genesis Block!"` + `"{m}:{s} · Day {d}"` (hud_ui.rs:1211, 1278). | One-per-world, fires on the **first Satori** ever mined in that world. Already framed in code comments as "like Bitcoin's block 0, there is only one ever" (save.rs:2342). | **Reframe the toast + audio cue copy as a *founding event*** — "your world's story begins" — not a treasure grab. Pure string change. |
| **Diamonds** | `block.rs`: `DIAMOND_ORE=18`, `DIAMOND_BLOCK=22`, `DEEPSLATE_DIAMOND_ORE=28`, `DIAMOND_CHEST=266`. `item.rs`: `MaterialId::Diamond` (item.rs:41), display `"Diamond"` (item.rs:640). `armour.rs`: `ArmourMaterial::Diamond` tier T4 (armour.rs:42), below `Satori` T5. | A **tool/armour material tier** (T4). **NOT a currency** today. | **Frame diamonds as "the old money" in lore copy only** (loading tips, world-creation line). The §3/§4 dual-role knot (diamonds-as-money + felt debasement) is a **mechanic** → DEFERRED. |
| **Villagers** | `villager.rs`: `Profession` enum (Farmer, Blacksmith, Cook, Scribe, Carpenter, Builder, Miller, Baker, Brewer); `VillagerComponent.gossip_line: Option<String>` daily-refreshed; gossip pools `GENERIC_GOSSIP` (villager.rs:254) + per-profession pools (villager.rs:273+). | Quest-giver / workstation-claimer skeleton with **daily gossip lines** (data-driven string pools). | **Add a few Diamond-Age-flavoured gossip lines** to the existing pools (pure data — new strings in the same `&[&str]` arrays). No behaviour change. The §4 "villagers as real economic actors" build is **Phase B**, out of scope. |
| **Proof of Play (PoP)** | `proof_of_play.rs::proof_hash(...)` (proof_of_play.rs:78); surfaced truncated to the player; loading tip "Proof of Play" already exists (loading_tips.json:3). | Always-on educational proof-of-work hash on every strike; drives the Genesis celebration + drops. | **No change.** PoP already fits the myth ("the work that secures the chain"). We may *optionally* add ONE lore-flavoured tip alongside the existing factual one — copy only. **No earning/sats framing.** |
| **Vendors** | `vendor.rs`: `VendorMode` (Sell/Buy/Barter/SellPlan*/Bulk); `VendorData`. No flavour text in the module — UI copy is rendered by callers (`vendor_ui.rs`, `hud_ui.rs`). | Player-placed trade stalls; functional, no narrative. | **Optional**: a single diegetic flavour line in the vendor UI ("the old money still spends here") — copy only, if a clean low-risk insertion point exists. Likely **DEFER to Phase B** with the living economy. |
| **World Integrity Ledger** | `hostile_acts.rs`: `HostileActLedger` (append-only `Vec<HostileAct>`, currently records `CartRobbery`). The *cheat* ledger (World Integrity) is referenced separately. | Append-only record; "what you build becomes history." | **No change in Phase A.** The "permanent legacy ledger" tie-in (§5) is conceptual only here; the build is Phase C. |
| **Loading tips** | `assets/loading_tips.json` (array of `{kind, title, body}`, `kind ∈ {tip, new}`); loaded via `loading_screen.rs::CARDS_JSON = include_str!("../assets/loading_tips.json")` (loading_screen.rs:15); parsed into `LoadingCard { kind: CardKind, title, body }`. **Note:** also mirrored at `dist/loading_tips.json` + `dist/.stage/loading_tips.json` — the build copies it. | 14 `tip` + 4 `new` cards, gameplay-guidance tone (e.g. "Chase the Satori", "Proof of Play"). | **Add 2–4 lore tips** in the existing JSON format/tone — the cheapest, safest theme vector. Pure data. |
| **World-creation flavour** | `menu.rs` new-world form (menu.rs:3198+): section labels `"GAME MODE"`, `"WORLD TYPE"`, mode/type buttons. Functional egui form, **no narrative line today**. | Plain config form. | **Add ONE short evocative line** near the top of the create-world panel ("Begin a new world in the Age of Diamonds…"). Pure copy (one `ui.label`). |

**Existing tone to match** (from `loading_tips.json`, for the new tips):
> `{ "kind": "tip", "title": "Chase the Satori", "body": "The orange gem hides deep in the deepslate — the rarest, toughest material in the world." }`
> `{ "kind": "tip", "title": "Proof of Play", "body": "Every strike runs a real hash. Watch it flash on each hit — that's the proof of your work." }`

Short, second-person, evocative-but-practical. UK English. The new lore tips sit naturally beside these.

---

## 3. Concrete, low-risk tasks

### MVP (pure copy/flavour — IN SCOPE, see §4)

**Task A — Reframe the Genesis Block celebration as a founding event.**
*Existing:* `game_loop.rs:7167` fires the toast `"Genesis Block! The first Satori of this world!"` (and the scenario end-card `"Genesis Block!"`, hud_ui.rs:1278).
*Change:* Edit the **toast string** (and, for consistency, the scenario end-card headline/detail) so it reads as the **founding of the player's world**, not a loot pickup. Candidate copy (final wording is the owner's call — keep it short, no "Bitcoin", no lecture):
- Toast: `"Genesis Block — your world's story begins."` (keep the 8 s duration + `play_genesis_block()` cue unchanged).
- Optional second toast line / sub-text if the toast supports it: `"The first Satori. There is only ever one."`
*Risk:* **None beyond text.** Same trigger, same flag, same audio, same save. One/two string literals.
*Note:* The §2 comment in code already calls it "like Bitcoin's block 0" — that's an internal code comment, fine; the *player-facing* string must stay Bitcoin-free, which it already is. Keep it that way.

**Task B — Add Diamond-Age lore tips to the loading pool.**
*Existing:* `assets/loading_tips.json` (and its `dist/` mirrors). 
*Change:* Append 2–4 `{ "kind": "tip", ... }` cards in the existing format/tone that seed the setting **diegetically** — the world runs on diamonds (the old money), and there is a founding to be earned. **No "Bitcoin", no "earn", no economics explainer.** Candidate cards (owner-tunable):
- `{ "kind": "tip", "title": "The Age of Diamonds", "body": "This frontier runs on diamonds — the old money. Mine them, trade them, hoard them… for now." }`
- `{ "kind": "tip", "title": "The first Satori", "body": "The deepest gem founds your world. Only one Satori is ever the first — the rest are just treasure." }`
- `{ "kind": "tip", "title": "A world to found", "body": "Mine, build, defend, prosper. A civilisation worth its founding isn't built in a day." }`
*Risk:* **None.** Data only. (Remember to keep `dist/loading_tips.json` in sync — the build copies the asset; verify it lands in the bundle.)

**Task C — One world-creation flavour line.**
*Existing:* `menu.rs:3198+` create-world panel — functional form, no narrative.
*Change:* Add a single short `ui.label` near the top of the panel (above or beside the name/seed fields) — e.g. `"Begin a new world in the Age of Diamonds."` — small, muted, evocative. One label, no logic.
*Risk:* **None beyond text + a trivial layout add.** Does not alter world creation behaviour.

**Task D (optional, MVP-eligible if clean) — One lore-flavoured PoP/villager string.**
- *PoP:* optionally add ONE additional lore tip beside the existing factual "Proof of Play" tip (Task B already covers the tip pool; this is just an extra card if desired) — copy only, no sats framing.
- *Villagers:* optionally append 2–3 Diamond-Age gossip lines to the existing `GENERIC_GOSSIP` / per-profession pools in `villager.rs` (e.g. *"Diamonds buy less bread than they did last season."* — note: that line *hints* at debasement but changes **no mechanic**; it's a rumour string, the same as the existing "Strange tracks in the woods" lines). Pure data — new `&str` entries in the existing arrays.
*Risk:* **None beyond text.** Only include in MVP if the insertion is a clean one-array-edit; otherwise roll into Phase B.

### DEFERRED — mechanic-touching (NOT in the MVP; playtest-gated)

These deliver the *felt* half of the myth but each changes **behaviour**, so per §1's one-line test they are out of the MVP. They are specced here so the seam is understood, and each carries the **§7-knot warning**.

**Task E (DEFERRED) — Felt diamond-value drift / "hoard erosion".**
*What:* Make a diamond hoard *feel* the old money's debasement — savings drift, prices rise — so the player viscerally learns "savings in bad money decay" (north-star §2, §5, §7-knot 7).
*Why deferred:* It is a **new economic rule** (touches drops/economy/vendor pricing or a hoard-decay mechanic) — a Phase C-class system, and the dual-role diamonds knot (§4/§7-knot 6) must be resolved first so **tools keep working while the money debases**.
**§7-knot warning (must read as insight/agency, NEVER punishment/rug-pull):** if a player's hoard erodes, it must land as *"get out of diamonds"* insight and agency — **never** as *"the game stole my stuff."* This is delicate tuning. **Do not ship without the voluntary-player fun test.** Do not put any version of this in the MVP.

**Task F (DEFERRED) — Diamonds-as-money dual role.**
*What:* Resolve §4/§7-knot 6 — diamonds as both useful material *and* money (gold-like), with manipulation hitting a *minted* diamond-currency rather than raw gems.
*Why deferred:* A registry/economy change, prerequisite for Task E. Phase B/C.

**Task G (DEFERRED) — Living villager economy + monetary-adoption metric.**
*What:* Villagers as real economic actors (mine/trade/take jobs/choose money), the adoption metric, "a villager finds Genesis."
*Why deferred:* **This is Phase B** — explicitly the biggest build and out of Phase A scope (north-star §8).

---

## 4. MVP scope (what we actually ship in Phase A)

The MVP is **Tasks A, B, C** (and optionally D if the insertions are one-edit-clean). Together they are:

- **Pure copy/flavour over existing assets** — string literals in `game_loop.rs` (+ `hud_ui.rs` for consistency), new JSON cards in `loading_tips.json`, one `ui.label` in `menu.rs`, optionally new `&str` entries in `villager.rs`.
- **Zero new core systems**, zero new save fields, **no save-format change**, no protocol change, **no behaviour change beyond text/flavour**.
- **Solo-verifiable** — it builds, tests stay green, and the only observable difference is wording.
- **Fun floor untouched** — mine/build/farm/defend/explore behave identically.

**Explicitly NOT in the MVP:** any value-drift/hoard-erosion mechanic (E), diamonds-as-money (F), the living villager economy / adoption metric (G), eras/halving, the manipulating power, real-time server worlds. Those are Phases B/C.

---

## 5. Acceptance criteria

**Solo-verifiable (no playtest needed):**
1. `./check.sh` is green — `cargo clippy`, `cargo build`, `cargo test --bin axenstax-engine`, `trunk build`, bundle-size gate all pass. (No new tests strictly required since behaviour is unchanged; if a string is asserted anywhere, update the assertion.)
2. **Genesis copy reframed:** mining the first Satori in a fresh world fires the celebration with the **new founding-event toast** (e.g. "your world's story begins"), same 8 s duration, same `play_genesis_block()` cue, same one-per-world behaviour. The scenario end-card (if reached) shows the consistent reframed headline.
3. **Loading tips:** the new Diamond-Age tip cards appear in the loading rotation (verify they parse — `loading_screen.rs` deserialises `loading_tips.json`; a malformed card fails the build/load) and that the **`dist/` mirror is in sync** so the WASM bundle ships them.
4. **World-creation line:** the create-world panel shows the single flavour line; world creation otherwise behaves identically (Normal/Blank Canvas, Survival/Creative all unchanged).
5. **Compliance grep:** no player-facing string added in this phase contains "Bitcoin", "crypto", "sats", "earn", or an economics-lecture sentence. (Grep the changed strings.)
6. **No save/protocol drift:** `save.rs` `WorldMeta` is unchanged; existing save round-trip tests still pass; no protocol version bump.

**Requires the voluntary-player fun test (NON-negotiable gate — north-star §8):**
7. **The theme reads as enrichment, not gating** — a non-coerced player (ideally a non-family kid) still has fun mining/building, *and* the founding framing lands as "cool, my world has a story" rather than confusing or preachy. This is a *vibe* check, not a unit test, and it is the gate before any Phase B/C cathedral work. (See `feedback_axolittle_coerced_not_fun_signal` — a coerced tester is a bug oracle, not a fun oracle.)

---

## 6. Explicitly deferred (NOT in scope)

- **The living villager economy (Phase B)** — villagers as real economic actors, the monetary-adoption metric, "a villager finds Genesis." The lynchpin and biggest build.
- **The full sim (Phase C)** — the manipulating power / fiat antagonist, era/halving progression, the standard war, optional real-time persistent server worlds.
- **Any value-drift / hoard-erosion mechanic** (Task E) and **diamonds-as-money dual role** (Task F) — carry the §7-knot warning; must read as insight/agency, never punishment/rug-pull; playtest-gated.
- **Eras / difficulty-adjustment / halving** as gameplay — north-star §5 keystones, Phase C.
- **Any earning/payout surface** — payouts deferred regardless; the (future) founding reward is **legacy/standing, never sats** (north-star §3, §9).

**The non-negotiable gate (carried verbatim from north-star §8):** none of the above — and not even the full felt-debasement half of Phase A — skips the **voluntary-player fun test**. The fun floor (§6 of the north-star) must be proven with non-coerced players before the cathedral goes up, *"or the cathedral has no congregation."*

---

## 7. Honest scope note

This spec is **theme and flavour only.** It buys ~80% of the *meaning* of the founding myth for ~20% of the build — by reframing the Genesis Block as a world event and seeding the Diamond Age in copy. The big, original, un-built work — the living economy (B) and the full monetary sim (C) — is **not** in this spec, and it does not begin until the fun floor is validated by voluntary players. Phase A is the cheap, reversible, fully-compliant first brushstroke; it must never be mistaken for the founding itself.
