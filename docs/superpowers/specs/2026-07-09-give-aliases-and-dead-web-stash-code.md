# Give-command Magnesium aliases + dead web-Stash code — cleanup spec

**Status: SHIPPED 2026-07-09.** Two small, independent, self-contained findings surfaced during the 2026-07-09 wiki + all-sites content audit (see `MEMORY.md` → `project_wiki_audit_2026_07_09` / `project_all_sites_audit_2026_07_09` for the audit's full context if useful, but this doc is self-contained — you don't need to read those). Neither was fixed at the time (docs-only task); this is the pickup.

**Outcome:** Task A done as specced (TDD'd). Task B's investigation found the blast radius much larger than scoped here — three Stash-toggle UI sites (not one), threaded through `save.rs`'s hot save path, `menu.rs`'s lobby state, and `game_loop.rs` — all confirmed dead for the same reason. Owner chose full removal over documentation. The removal was scoped to what's exclusively tied to the dead push/sync path; the read-only cloud-world-list merge (`menu.rs::poll_cloud_worlds`) and the cosmetics/wardrobe Blossom sync in `cloud.js` were deliberately left in place (also inert, but shared with live/out-of-scope code — removing them risked the load-bearing local world-list path for no behaviour change). See commit for the full file list.

## TL;DR

1. **Task A (trivial, do first):** `/give` has zero aliases for the entire Magnesium item family. Add them to `give.rs`, following the file's own established convention. TDD it.
2. **Task B (investigate → decide → act):** `tools/sites/game/static/cloud.js` + its Rust-side wiring implement a web-build cloud-save path that appears to be unreachable today (its trigger UI is native-only-gated, and its precondition — a signed-in persona — has no path on web since web login was retired). Confirm that's actually true end-to-end, then either delete the dead path or leave a clear doc comment explaining why it's intentionally kept (e.g. staged groundwork for a future non-anonymous web tier). Do NOT assume either answer — trace it and see what's actually true before touching anything.

Both tasks are independent; do them in either order, but A is a 10-minute fix and B needs real investigation, so front-load A for a quick win.

---

## Task A — `/give` Magnesium family aliases

### The bug

`game/engine/src/commands/builtins/give.rs::material_by_name()` (starts line 207) has zero entries for `MaterialId::Magnesium`, `MaterialId::Fertiliser`, `MaterialId::Sparkler`, `MaterialId::Flare`, or `MaterialId::MagnesiumFirestarter` — confirmed by `grep -n "magnesium\|Magnesium" game/engine/src/commands/builtins/give.rs` returning nothing. All five materials are real, defined in `game/engine/src/item.rs` around line 283-288 (Spec 37, "Magnesium... Mineral + its products"), with working crafting recipes. They're just impossible to `/give` for testing — a real, if minor, gap in dev/testing tooling and in-game `/give`-based cheats.

This was caught because `docs/learn-journey/light-a-blasting-keg.md` told players to run `/give magnesium_firestarter`, which currently errors with "unknown item." That doc has already been fixed to route around the bug via the real crafting recipe instead — this spec is about fixing the actual gap, not the doc (the doc fix already shipped in `main@a6ba0a01`).

### The fix

Add a new arm to `material_by_name()` in `game/engine/src/commands/builtins/give.rs`. Follow the file's existing convention exactly — look at the `"sulphur" | "sulfur" => Some(M::Sulphur)` block around line 211 for the pattern (comment naming the spec, `snake_case` primary alias plus common misspellings/shorthand as `|`-separated alternatives). Suggested aliases, but use judgement to match the file's existing naming style for the rest:

```rust
// Spec 37 — Magnesium. Mineral + its products.
"magnesium" => Some(M::Magnesium),
"fertiliser" | "fertilizer" => Some(M::Fertiliser),
"sparkler" => Some(M::Sparkler),
"flare" => Some(M::Flare),
"magnesium_firestarter" | "firestarter" | "fire_starter" => Some(M::MagnesiumFirestarter),
```

A natural insertion point is right after the `"sulphur"`/`"saltpetre"`/`"black_powder"` Spec 49 block (line ~211-213), since Magnesium is a sibling mineral-and-products family from the neighbouring spec — but don't force it there if a different spot reads more naturally once you're looking at the current file.

### Acceptance criteria (TDD it — write the failing test first)

- A test in `give.rs`'s existing test module (or wherever `material_by_name` is already tested — check first) asserting `material_by_name("magnesium") == Some(MaterialId::Magnesium)` and the same for the other four, fails before the fix and passes after.
- `cargo test --bin axenstax-engine` green.
- `cargo clippy --bin axenstax-engine -- -D warnings` clean.
- Manually sanity-check in a running dev build if convenient: `/give magnesium 5` should now work (not required, the unit test is sufficient proof).

### Out of scope

Don't go hunting for every other possibly-missing `/give` alias across the whole material set — this spec is scoped to the five Magnesium-family materials that were actually confirmed missing. If you notice other obvious gaps while you're in there, note them for a future pass rather than scope-creeping this one.

---

## Task B — the web-Stash `cloud.js` dead-code question

### What's been established so far (verified, not guessed)

- `tools/sites/game/static/cloud.js` (920 lines) implements a **serverless** cloud-save path: the browser talks directly to a Blossom (Nostr blob-storage) server using the signed-in persona's own key (BUD-02 auth) and to the `trotters` relay for a player-signed encrypted manifest. The game server (`tools/sites/game/app.py`) never sees world data — it only tells the client where Blossom lives, via `BLOSSOM_PUBLIC_URL` (env var, currently unset in production, which makes `_cloud_save_enabled()` at `app.py:114-118` return `False`).
- It's wired into the Rust/WASM side: `game/engine/src/wasm_save.rs` has `#[wasm_bindgen]` extern bindings (`sync_stash_start`/`sync_stash_cancel`/`sync_stash_status`, `set_cloud_save_wasm`, `cloud_save_wasm`, `cloud_available` — see `wasm_save.rs:164-172, 370-391` and surrounding) that call into `cloud.js`'s exported functions by name (`axenstax_sync_stash_start` etc.).
- `game/engine/src/menu.rs`'s `CardAction::ToggleStash` handler (around line 923-951) calls `set_cloud_save_wasm`/`cloud_save_wasm` inside a `#[cfg(target_arch = "wasm32")]` block. The `ToggleStash` match arm itself is NOT cfg-gated (unlike its neighbours `Export`/`ExportToFolder`, which are explicitly `#[cfg(not(target_arch = "wasm32"))]`) — so the *code* would run on a wasm32 build if the action ever fired.
- BUT: the world-card Stash toggle **button** that would dispatch `CardAction::ToggleStash` (`stash_button_style`/`stash_hint` in `menu.rs`, around lines 3122/3147) IS `#[cfg(not(target_arch = "wasm32"))]`-gated — i.e. that button doesn't exist in a web build. On web, the lobby shows a `desktop_feature_button(ui, "☁ Stash")` instead, which opens a "get the desktop app" explainer dialog, not a real toggle.
- Web login (the only way to get a signed-in persona/signer on web, which the Blossom auth requires) was retired 2026-06-27 (`app.py:703`: "Web login was retired... the web build is a login-free local sandbox").
- This is the same *shape* of problem as `tools/sites/game/templates/request_access.html`, which was confirmed-dead (zero references anywhere) and deleted in `main@a6ba0a01` — a flow got retired and its supporting code was never cleaned up. The difference is `request_access.html` was trivially provably dead (grep found zero callers); this one is one layer more indirect (the code is *wired* and *reachable in principle*, but its trigger button doesn't exist on the target it would need to run on).

### What hasn't been nailed down — do this before touching anything

1. **Confirm the toggle button really is the only trigger.** Grep `menu.rs` and anywhere else `CardAction::ToggleStash` is constructed/dispatched from — is the native-only-gated button genuinely the *only* way to fire it, or is there another path (a keyboard shortcut, a different menu, a command)?
2. **Confirm there's no other way to get a signer on web.** The `cloud_available()` check in `wasm_save.rs` presumably checks for *some* signer — trace what it actually checks. If it's strictly gated on the retired web-login flow, that closes the loop. If there's any other path to a signer on web (e.g. a native-companion-app bridge, a QR-pair flow, anything), this changes the answer.
3. **Check whether this is intentional staged groundwork rather than an oversight.** `docs/foundations/2026-05-26-cloud-save-blossom.md` is the original design doc for this feature — read it. If it explicitly frames this as a "destination tier" for a *future* signed-in web experience (distinct from today's anonymous taster), that's a product decision to surface to the owner, not something to unilaterally delete. Check project memory `project_web_taster_compliance_decision.md` too — the "no login/Stash on web" rule is described there as a compliance decision, and this code predates or straddles that decision (design doc is 2026-05-26; web-login-retirement is 2026-06-27), so it's plausible this is simply orphaned rather than intentional.

### Decide, then act

- **If genuinely unreachable and not intentional groundwork:** remove `cloud.js`, its imports from `beacon.js`/`mailbox.js`/`gamestr.js` (whichever actually import it — check first, don't assume), the `wasm_save.rs` extern bindings and their Rust callers in `menu.rs`, and the `BLOSSOM_PUBLIC_URL`/`_cloud_save_enabled()`/CSP-slot plumbing in `app.py`. This is a bigger, more surgical removal than the `request_access.html` delete — go slowly, and run `cargo build --target wasm32-unknown-unknown` + `check.sh` (native + WASM parts) after, since this touches real wasm_bindgen wiring that other code may transitively depend on.
- **If it's intentional staged groundwork for a future feature:** leave the code as-is, but add a clear doc comment at the top of `cloud.js` and above `_cloud_save_enabled()` in `app.py` stating explicitly that this is inert-by-design pending a product decision (no web login path exists today), so the next person who finds it doesn't have to re-derive all of this from scratch.
- **If genuinely unclear / a real product call:** don't guess — write up what you found (steps 1-3 above) and flag it to the owner rather than deciding unilaterally. This is the kind of thing where guessing wrong in either direction (deleting staged work vs. leaving a real dead-code/compliance-adjacent gap) is worse than asking.

### Acceptance criteria

- Steps 1-3 above are actually traced through the code (not assumed) and the findings are written down (in the PR/commit description at minimum).
- Whichever action is taken (remove / document / flag-to-owner), `check.sh` stays green (clippy `-D warnings`, native build, all tests, WASM trunk build) if any code changed.
- If code was removed: no leftover references (`grep -rn "cloud\.js\|BLOSSOM_PUBLIC_URL\|sync_stash\|cloud_save_wasm" tools/sites/game/ game/engine/src/` should only show what's still genuinely used, if anything).

## Memory-rule check

- **Regulatory red line (CLAUDE.md, repo root):** "No central collection of kids' data... Web taster stays anonymous/local (no login, cookies, analytics, or age data)." Task B is directly downstream of this rule — if step 1/2 above finds a live path that lets a web player's data reach a cloud store, that's not just a code-cleanup question, it's a compliance question, and should be flagged loudly rather than quietly fixed.
- **`feedback_confirm_before_ci_spend`:** this repo's CI Actions minutes are metered — neither task here should need `native-packages.yml` or macOS/Windows CI, but if Task B's removal touches anything that would trigger those, confirm with the owner first per that standing rule.
- **`feedback_merge_to_main_preauthorised`:** standing permission to merge/push AxeNStax PRs without asking, healthy-gate only — applies once `check.sh` is green.
- **Do NOT build anything not explicitly asked for** (CLAUDE.md Rules section) — Task B's "decide, then act" branch should stay narrowly scoped to the cleanup decision; don't use this as an excuse to build the "future signed-in web tier" if that's what you find — just flag it.
