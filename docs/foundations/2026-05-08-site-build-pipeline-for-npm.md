# AxeNStax site build pipeline — bring `nostr-tools/nip46` into vanilla `auth.js`

**Status**: DELIVERED 2026-05-08 on `main`. Vendored bundle at `tools/sites/game/static/vendor/nostr-tools-nip46.iife.js` (70.1 kB minified). Wired into `game/engine/index.html` (always loaded post-WASM-boot path) and `tools/sites/game/templates/lobby.html` (post-sign-in only). Smoke-verified in node: `window.AxeNostrNip46.{BunkerSigner, parseBunkerInput}` both present. Specs 8 (pairing storage) and 9 (alpha onboarding) now unblocked.
**Date**: 2026-05-08
**Memory rules in scope**: signet boundary (this is AxeNStax-internal infra, not a Signet-side ask), autonomy to playtest boundary (the recommended path is solo-verifiable end-to-end on the dev box).

---

## TL;DR

`tools/sites/game/static/auth.js` is plain vanilla browser JS — IIFE-style, loaded via `<script src="/static/X.js">` from `lobby.html`, no bundler, no `import` statements. Charter Phase 1 needs `BunkerSigner` from `nostr-tools/nip46`, which is ESM-only and has a transitive dep tree (`@noble/*`, `@scure/*`).

**Recommended path**: vendor a *built artefact* — run `esbuild` once against `nostr-tools/nip46` with `--bundle --format=iife --global-name=AxeNostrNip46`, commit the output to `tools/sites/game/static/vendor/nostr-tools-nip46.iife.js`. `auth.js` reads `window.AxeNostrNip46.BunkerSigner`. No permanent build pipeline, no runtime CDN dependency, matches the existing vendor-locally pattern.

Cost: ~half a day to set up + verify. Re-run the build on `nostr-tools` upgrade (rare in alpha).

---

## Context pointers

- Existing static-asset loading pattern: `tools/sites/game/templates/lobby.html:157-165` — every external lib is a hand-vendored `static/*.js` file (`noble-curves.js`, `bech32.js`, `qrcode-generator.js`, `signet-verify.js`).
- Existing IIFE pattern: `tools/sites/game/static/auth.js:19` (`(function () { 'use strict'; ...`)
- Existing IDB pattern (for the pairing-storage spec): `tools/sites/game/static/world_store.js`.
- Charter Phase 1 punch list referencing `nostr-tools/nip46`: `docs/foundations/2026-05-08-charter-integration-axenstax.md` §"Phase 1 — WASM JS bridge…"
- Upstream Charter spec: `~/Documents/<workspace>/forgesworn/signet-plans/docs/plans/2026-05-08-charter-schedule-clause-spec.md` §Q5 Phase 1 "Client library" rev. 3 answer.

---

## Constraints

- **Vanilla / IIFE pattern is load-bearing** for the alpha posture. `auth.js` is read frequently by humans for security-sensitive work; preserving plain JS keeps the diff legible.
- **No new runtime dependencies for the alpha launch.** External CDN dependencies (esm.sh, jsdelivr, unpkg) introduce runtime failure modes the alpha can't absorb.
- **No permanent build pipeline** unless the alternatives genuinely don't work. AxeNStax's site already starts with a single shell script (`tools/sites/game/start.sh`); adding webpack/vite for one library is over-engineering.
- **`BunkerSigner` shape**: per the Charter spec, AxeNStax uses `BunkerSigner.fromBunker(uri)` from a `bunker://` URI and calls `signer.sendRequest('charter_check', [...])`. That's the only API surface needed for Phase 1.

---

## Three options evaluated

### Option A — Vendor a built artefact (RECOMMENDED)

Run `esbuild` once against `nostr-tools/nip46` with the IIFE format:

```bash
cd tools/sites/game/static/vendor
npx -y esbuild@0.21 \
  --bundle \
  --format=iife \
  --global-name=AxeNostrNip46 \
  --target=es2022 \
  --minify \
  --legal-comments=external \
  ../scratch/nostr-tools-entrypoint.js \
  > nostr-tools-nip46.iife.js
```

Where `scratch/nostr-tools-entrypoint.js` is a tiny re-export shim:

```js
// scratch/nostr-tools-entrypoint.js — input to esbuild only
import { BunkerSigner, parseBunkerInput } from 'nostr-tools/nip46';
export { BunkerSigner, parseBunkerInput };
```

Commit `nostr-tools-nip46.iife.js` into `tools/sites/game/static/vendor/`. Add a script tag to `lobby.html`. `auth.js` reads `window.AxeNostrNip46.BunkerSigner`.

**Pros**:
- No runtime CDN dependency.
- Matches existing vendor-locally pattern.
- No permanent build step in `start.sh` — the bundle is a committed artefact regenerated on `nostr-tools` upgrades.
- Bundle size is bounded (`nostr-tools/nip46` minified is ~80 KB) and loads in parallel with other static assets.

**Cons**:
- One-time build setup (~1 hour to nail down the entrypoint shim + the esbuild invocation).
- A committed `.iife.js` artefact is a slightly unusual git artefact — needs a one-liner in the file header explaining how to regenerate.
- Upgrade discipline: when `nostr-tools` upgrades, regenerate + commit. Cheap per-upgrade.

### Option B — Runtime ESM via `<script type="module">` from CDN

`lobby.html` adds:

```html
<script type="module">
  import { BunkerSigner, parseBunkerInput } from 'https://esm.sh/nostr-tools@2.23.3/nip46';
  window.AxeNostrNip46 = { BunkerSigner, parseBunkerInput };
  window.dispatchEvent(new Event('nostr-tools-ready'));
</script>
```

`auth.js` listens for `nostr-tools-ready` before initialising any Charter pairing flow.

**Pros**:
- Zero build step.
- Always-current version (within a pinned semver).

**Cons**:
- Runtime CDN dependency — esm.sh outage breaks the Charter pairing flow at runtime.
- Module scripts are deferred-by-default and have different load semantics than IIFEs — `auth.js` must accommodate the async window-property publication.
- Subresource integrity is awkward across a transitive dep tree esm.sh dynamically resolves.

### Option C — Hand-vendor `nostr-tools/nip46` + transitive deps

Walk the dep tree manually, copy each file into `static/vendor/`, hand-edit `import` statements to relative paths.

**Pros**:
- Maximum transparency.

**Cons**:
- Error-prone (typed deps, conditional exports, version-pinning).
- Painful upgrades.
- The whole reason a bundler exists.

Rejected — option A buys vendoring's reliability without the maintenance pain.

---

## Recommendation

**Option A.** Build artefact + commit. Reasoning:

1. Preserves the alpha's no-runtime-CDN posture.
2. Matches the existing vendor-locally pattern more cleanly than B.
3. The "regenerate on upgrade" discipline is cheap because `nostr-tools` is a stable mature library — upgrades will be rare during alpha.
4. Falls back gracefully across browsers (no module-script load-order coordination needed).

If a permanent build pipeline lands later for *other* reasons (e.g., shipping multiple sites with shared assets), this spec converts to "include nostr-tools/nip46 in the canonical build" without rework — the input-to-esbuild shim is reusable.

---

## Phased plan (option A)

### Phase 1 — Prove the bundle (~2 hours)

- Create `tools/sites/game/static/vendor/scratch/nostr-tools-entrypoint.js` (the re-export shim).
- Run `esbuild` per the command above. Output `nostr-tools-nip46.iife.js`.
- Open `lobby.html` in a browser with the new script tag. Verify `window.AxeNostrNip46.BunkerSigner` is present.
- Smoke: `BunkerSigner.fromBunker('bunker://foo')` should not throw on construction (will error on actual relay, fine for smoke).

### Phase 2 — Wire into `lobby.html` (~30 min)

- Add `<script src="/static/vendor/nostr-tools-nip46.iife.js"></script>` to `lobby.html`, before `auth.js` (so it's available at module init).
- Add the same to any other templates that load `auth.js`.

### Phase 3 — Add upgrade discipline (~30 min)

- Add a `tools/sites/game/static/vendor/REGENERATE.md` with the exact `esbuild` command + version pin.
- Add a header comment to `nostr-tools-nip46.iife.js` (just a regenerate-instructions one-liner pointing at REGENERATE.md).
- Add an entry to `CLAUDE.md` "Spin Up" section noting the vendor-bundle exists.

### Phase 4 — Verification gate

- `./check.sh` green (no behaviour change in the engine).
- `./check.sh --smoke` green — the smoke test loads `lobby.html` and confirms WASM boots; the new vendor script must not break that path.
- Bundle size check: total static-asset bundle-size delta < 100 KB minified.

---

## Acceptance

- `auth.js` (or any static script after the new vendor file loads) can read `window.AxeNostrNip46.BunkerSigner` and `parseBunkerInput`.
- `nostr-tools-nip46.iife.js` is committed to the repo with a regeneration-instructions header.
- No new runtime dependency on esm.sh or any CDN.
- No permanent build step in `start.sh`.
- `./check.sh --smoke` green.

---

## What this spec does *not* cover

- The `auth.js` wiring that *uses* `BunkerSigner` — that's `2026-05-08-charter-pairing-storage.md` and the Charter Phase 1 implementation work itself.
- Onboarding UX for parents — that's `2026-05-08-alpha-onboarding-charter-pairing.md`.
- Any cross-platform crate work — that's Charter Phase 2 (Forgesworn-leading per the Charter spec Q5).

---

## Memory rules check

- signet boundary — this is AxeNStax-internal site infrastructure, not a Signet-side ask. ✓
- autonomy to playtest boundary — the entire Phase 1–4 plan is solo-verifiable on the dev box. No playtest gate. ✓
- pwa priority — bundling for PWA delivery is on-pattern. ✓
