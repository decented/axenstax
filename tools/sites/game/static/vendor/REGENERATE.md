# Vendor bundles — regenerate

This dir holds **built artefacts** vendored into the AxeNStax site. They are committed (not git-ignored) so the site stays buildable without a permanent build pipeline. Regenerate when upgrading the underlying package.

## `nostr-tools-nip46.iife.js`

**What it is**: exposes `BunkerSigner` + `parseBunkerInput` from `nostr-tools/nip46` as `window.AxeNostrNip46.{BunkerSigner, parseBunkerInput}` — the NIP-46 "remote signer" machinery that lets the site ask a player's Signet **bunker** to sign an event *without the secret key ever leaving the bunker* (think: "approve this in your phone app", like a banking-app confirmation).

**Status (2026-05-27): DORMANT — no live consumer.** Loaded by `game/engine/index.html`, but nothing calls it today. Its original consumer — Charter Phase 1 — was removed in the Charter strip. It is **deliberately retained** (decision: keep, owner-confirmed 2026-05-27) for the next planned consumer:

- **Multiplayer identity — Spec 1 Phase 4.** When a player joins an online/LAN game, prove who they are by having their Signet sign the join challenge, instead of trusting a client-typed name (closes the `JoinRequestPacket.player_name` trust gap / Spec 8 §9.0.1 T-NP-LEAK). The wiring is specced in **`docs/foundations/2026-05-14-engine-signing-bridge.md`** ("engine signing bridge", Sentinel AS-005): its `auth.js` hook routes through this bundle's `BunkerSigner.signEvent`.
- That spec is **gated on upstream Signet** shipping a generic `sign_event` NIP-46 handler (currently deferred to the Phase 5 Bitcoin lead-in). Until upstream lands it, this bundle stays unused — it is *staged*, not *orphaned*.

**To remove** (if that path is ever dropped or deferred indefinitely): delete this file + its `.LEGAL.txt` + the two `<script src=".../nostr-tools-nip46.iife.js">` includes (`game/engine/index.html`), then re-vendor via the recipe below when needed — a one-command rebuild.

**Source pin**: `nostr-tools@2.23.3`. Update the version in `scratch/package.json` when upgrading.

**Specs**: next consumer → `docs/foundations/2026-05-14-engine-signing-bridge.md`; vendoring pipeline → `docs/foundations/2026-05-08-site-build-pipeline-for-npm.md`.

### Regenerate

```bash
cd tools/sites/game/static/vendor/scratch
npm install --no-audit --no-fund --silent
./node_modules/.bin/esbuild \
  --bundle \
  --format=iife \
  --global-name=AxeNostrNip46 \
  --target=es2022 \
  --minify \
  --legal-comments=external \
  --outfile=../nostr-tools-nip46.iife.js \
  nostr-tools-entrypoint.js
{ echo "// vendored bundle — regenerate per ./REGENERATE.md (do not hand-edit). Source: nostr-tools/nip46@$(node -p 'require(\"./node_modules/nostr-tools/package.json\").version').";
  cat ../nostr-tools-nip46.iife.js;
} > ../nostr-tools-nip46.iife.js.tmp && mv ../nostr-tools-nip46.iife.js.tmp ../nostr-tools-nip46.iife.js
```

That produces:
- `nostr-tools-nip46.iife.js` — committed
- `nostr-tools-nip46.iife.js.LEGAL.txt` — committed (lib licenses)

### Verify after regenerate

1. Bundle is < 200 kB minified.
2. `head -1 nostr-tools-nip46.iife.js` shows the regenerate-pointer comment.
3. Open `https://localhost:8094/` (the game page at `/`), DevTools console:
   ```js
   typeof window.AxeNostrNip46?.BunkerSigner === 'function'
   ```
   Should print `true`.

### Why a bundle and not a CDN script

See the spec's "Three options evaluated" section. Short version: vendoring a built artefact preserves the site's no-runtime-CDN posture and matches the existing vendor-locally pattern (`noble-curves.js`, `bech32.js`, etc. are all hand-vendored).

## `signet-login.iife.js`

**Purpose**: drop-in "Sign in with Signet" SDK — extends `window.Signet` with `login`, `restoreSession`, `handleRedirectCallback`, `handleCallback`, `logout`. Forgesworn-maintained; lifts cross-game.

**CONSOLE ONLY as of 2026-09-27** (audit fix). The console still uses it for its
real operator login. The **game site no longer vendors or loads this bundle at
all** — the web taster is a login-free anonymous local sandbox (rewritten
2026-06-27; `auth.js` never made an `/auth` round-trip since, and this SDK
loading unused on a kids' surface was the audit finding). Its old
`/auth/callback` / `/auth/relay-complete` / `/auth/verify-fragment` server
endpoints are long gone too (superseded by the console's own `auth.py`).

**Source pin**: `signet-login@0.17.3` (npm), pinned by version **and** tarball
sha256 in `tools/sites/vendor-signet-login.mjs` (`PINNED_VERSION` /
`PINNED_TARBALL_SHA256`) — update both constants together when bumping, after
reviewing the diff. (Earlier history: 0.9.14 QR/relay auth-only fallback,
commit `d785700`; 0.9.15 bunker sign_event empty-pubkey, commit `cdc9373`;
0.9.15 → 0.12.1 → 0.13.1 on 2026-06-21.)

> **The deploy auto-vendors the pinned version — you rarely need to do this by
> hand.** `deploy.yml` runs `tools/sites/vendor-signet-login.mjs` before the
> build, which `npm pack`s the pinned version, verifies its tarball sha256
> against the pin, and writes it into the **console** vendor dir only (a deploy
> never breaks if npm is briefly unreachable, or refuses loudly if the sha256
> doesn't match — either way it falls back to the committed bundle). The
> committed copy in the repo is that **offline / local-dev fallback**; refresh
> it with the steps below when re-pinning a new version (e.g. so
> `signet-compatibility.yml`'s hard drift check is happy against the new pin).

**Cutover spec**: `docs/integrations/signet/2026-05-20-signet-login-adoption.md`.

> **IMPORTANT — vendor the npm `dist` byte-for-byte.** The drift-guard
> (`tools/sites/check-signet-login-bundle.mjs`, run by `deploy.yml` +
> `signet-compatibility.yml`) compares the vendored console copy's SHA-256 to
> the `dist/signet-login.iife.js` inside the pinned `npm pack
> signet-login@<version>`. So copy the **published artefact verbatim — no
> prepended header comment**. Building from the sibling source repo will not
> byte-match the npm publish and will fail the guard.

### Regenerate

```bash
VERSION=0.17.3   # bump deliberately — review the diff before re-pinning
TMP=$(mktemp -d)
TARBALL=$(npm pack "signet-login@${VERSION}" --pack-destination "$TMP" | tail -1)
sha256sum "$TMP/$TARBALL"   # update PINNED_TARBALL_SHA256 in vendor-signet-login.mjs to match
tar -xzf "$TMP/$TARBALL" -C "$TMP"
cp "$TMP/package/dist/signet-login.iife.js" \
   "<repo>/tools/sites/console/static/vendor/signet-login.iife.js"
rm -rf "$TMP"
```

That produces:
- `signet-login.iife.js` — committed in the `console/` vendor dir only (~343 kB; includes `@noble/curves`, `@noble/hashes`, `nostr-tools`, `qrcode`, `signet-verify`)

### Verify after regenerate

1. Bundle is < 400 kB (qrcode + nostr-tools + noble make it chunkier than the others; ~343 kB at 0.12.1).
2. `node tools/sites/check-signet-login-bundle.mjs tools/sites/console/static/vendor/signet-login.iife.js` prints "match" and exits 0.
3. Open the console's sign-in page, DevTools console:
   ```js
   typeof window.Signet?.login === 'function'
   typeof window.Signet?.handleRedirectCallback === 'function'
   typeof window.Signet?.restoreSession === 'function'
   ```
   All three should print `true`.

### Why a bundle and not the CDN

Same as the other vendored bundles — preserves the no-runtime-CDN posture so the site keeps working in air-gapped / local-dev environments and doesn't depend on `cdn.signet.forgesworn.dev` uptime.

## `scratch/`

Build inputs for the vendored bundles above. **Not shipped** — `node_modules/` is git-ignored at this scope, the `package.json` and entrypoint shim are committed so future regenerates use the same inputs.

## `stash.iife.js`

**Purpose**: the [`@forgesworn/stash`](https://github.com/forgesworn/stash) SDK as an
IIFE (`window.AxeStash`) — per-persona encrypted save vault (NIP-44 encrypt-to-self
blobs on Blossom + per-persona manifest). Consumed by `static/cloud.js`
(`window.AxeCloud`) to give worlds "sign in anywhere, get your saves".

**Source pin**: `@forgesworn/stash` from sibling repo `../../../../../forgesworn/stash`
(built `dist/`). Bump by re-installing that path in `scratch/`.

```bash
cd tools/sites/game/static/vendor/scratch
# rebuild the SDK first if it changed:  (cd ../../../../../../forgesworn/stash && npm run build)
npm install ../../../../../../forgesworn/stash --no-audit --no-fund
VERSION=$(grep '"version"' node_modules/@forgesworn/stash/package.json | head -1 | sed -E 's/.*"version": *"([^"]+)".*/\1/')
./node_modules/.bin/esbuild \
  --bundle \
  --format=iife \
  --global-name=AxeStash \
  --target=es2022 \
  --minify \
  --legal-comments=external \
  --outfile=../stash.iife.js \
  stash-entrypoint.js
{ echo "// vendored bundle — regenerate per ./REGENERATE.md (do not hand-edit). Source: @forgesworn/stash@${VERSION}"; cat ../stash.iife.js; } > ../stash.iife.js.tmp && mv ../stash.iife.js.tmp ../stash.iife.js
```

Entrypoint: `scratch/stash-entrypoint.js`. Committed outputs: `stash.iife.js` + `stash.iife.js.LEGAL.txt`.
Loaded in `game/engine/index.html` before `cloud.js`.

## `relay.iife.js`

**Purpose**: a minimal Nostr relay client (`window.AxeRelay.makeRelayClient`) over
nostr-tools `SimplePool` — `publish(event)` + `query(pubkey, kind, dTag?)` +
`queryByTag(tag, value, kind, since)`, shaped for `@forgesworn/stash`'s
`nostrManifestStore` RelayClient. Used by `static/cloud.js` for the serverless
cloud-save manifest. (`static/mailbox.js`, the lobby mailbox's inbound NIP-17
pull, was removed with the web feedback channel on 2026-10-01; `queryByTag` is
kept in the bundle, which is a vendored build.)

**Source pin**: `nostr-tools@2.23.3` (already in `scratch/`).

```bash
cd tools/sites/game/static/vendor/scratch
./node_modules/.bin/esbuild \
  --bundle \
  --format=iife \
  --global-name=AxeRelay \
  --target=es2022 \
  --minify \
  --legal-comments=external \
  --outfile=../relay.iife.js \
  relay-entrypoint.js
{ echo "// vendored bundle — regenerate per ./REGENERATE.md (do not hand-edit). Source: nostr-tools/pool@2.23.3"; cat ../relay.iife.js; } > ../relay.iife.js.tmp && mv ../relay.iife.js.tmp ../relay.iife.js
```

Entrypoint: `scratch/relay-entrypoint.js`. Committed: `relay.iife.js` + `.LEGAL.txt`.
Loaded in `game/engine/index.html` before `cloud.js`.

## `beacon.iife.js`

**Purpose**: the [`@forgesworn/beacon`](https://github.com/forgesworn/beacon) SDK as an
IIFE (`window.AxeBeacon`) — public-content **distribution** primitive: signed,
plaintext, content-addressed items a persona publishes for anyone to discover and
adopt (the public sibling of `@forgesworn/stash`). Surface: `createBeacon`,
`verifyEvent`, `BEACON_MANIFEST_KIND` (kind 30820). Consumed by a future
`static/beacon.js` glue (Workshop sharing) — Task D1·T2.

**Source pin**: `@forgesworn/beacon` from sibling repo
`<workspace>/forgesworn/beacon` (built `dist/`). Bump by re-installing
that path in `scratch/`.

```bash
cd tools/sites/game/static/vendor/scratch
# rebuild the SDK first if it changed:
(cd <workspace>/forgesworn/beacon && npm run build)
npm install <workspace>/forgesworn/beacon --no-audit --no-fund
VERSION=$(grep '"version"' node_modules/@forgesworn/beacon/package.json | head -1 | sed -E 's/.*"version": *"([^"]+)".*/\1/')
./node_modules/.bin/esbuild \
  --bundle \
  --format=iife \
  --global-name=AxeBeacon \
  --target=es2022 \
  --minify \
  --legal-comments=external \
  --outfile=../beacon.iife.js \
  beacon-entrypoint.js
{ echo "// vendored bundle — regenerate per ./REGENERATE.md (do not hand-edit). Source: @forgesworn/beacon@${VERSION}"; cat ../beacon.iife.js; } > ../beacon.iife.js.tmp && mv ../beacon.iife.js.tmp ../beacon.iife.js
```

Entrypoint: `scratch/beacon-entrypoint.js`. Committed outputs: `beacon.iife.js` +
`beacon.iife.js.LEGAL.txt`.
Loaded in `game/engine/index.html` after `relay.iife.js`.

### Verify after regenerate

1. Bundle is < 100 kB minified (~34 kB; pulls in `@noble/curves` + `@noble/hashes`).
2. `head -1 beacon.iife.js` shows the regenerate-pointer comment with the right version.
3. Open `https://localhost:8094/`, DevTools console:
   ```js
   typeof window.AxeBeacon?.createBeacon === 'function'
   typeof window.AxeBeacon?.verifyEvent === 'function'
   window.AxeBeacon?.BEACON_MANIFEST_KIND   // 30820
   ```
   The first two print `true`. (Note: the `node -e "globalThis.window={};…"`
   one-liner used to spot-check stash *does not work* for these IIFEs under Node —
   esbuild's `var AxeBeacon = …` only attaches to the global object in a real
   browser, not a plain `window` object in Node's direct `eval`. Use indirect eval
   instead: `node -e "const f=require('fs').readFileSync('beacon.iife.js','utf8');(0,eval)(f);console.log(typeof AxeBeacon.createBeacon, typeof AxeBeacon.verifyEvent)"` → `function function`.)
