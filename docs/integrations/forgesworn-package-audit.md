# Forgesworn package consumption

Checked against the Forgesworn repositories and public npm registry on
15 October 2026. Prefer a published package with an exact version and recorded
integrity wherever the required API is available. A local `package.json` version
is not evidence that a package has been published.

| Consumer | Package / decision |
|---|---|
| Operator-console sign-in | Published `signet-login@0.19.1`; copy its shipped `./iife` browser export, without building SDK source. Version and reviewed tarball SHA256 are shared by deployment, refresh and drift checks. |
| Room wrapper identity utilities | Own lockfile in `tools/room/`; `nostr-tools@2.25.2` and `@noble/hashes@2.0.1` through public exports, replacing imports from a KithMoot checkout's `node_modules`. |
| Feedback reader and feedback tests | Already consume npm `nostr-tools` with their own manifests and lockfiles. No unrelated version change is needed. |
| Release tooling | Already consumes npm `nostr-tools` with its own manifest and lockfile. |
| Beacon offline-catalogue bake | `@forgesworn/beacon` returns 404 from the registry. Keep the explicitly configured built-SDK exception until a compatible release exists; do not add a non-existent npm dependency. |
| Stash | `@forgesworn/stash` returns 404. The browser taster no longer loads Stash; native storage uses Rust and is outside npm. |
| Room keeper | Neither `kithmoot` nor `@forgesworn/kithmoot` is published. Keep the explicit built-KithMoot exception for `RoomAgent`, identity, keeper state and ownership APIs; the room's npm utilities do not replace this library. |
| Native Signet signer and contacts | Rust dependencies / wire implementations, not JavaScript consumers. An npm package is not a compatible replacement. The vendored signer also has documented native pairing changes; keep them until a separately reviewed Rust replacement provides parity. |

Other published JavaScript packages include
`signet-verify`, `signet-protocol`, `nsec-tree`, `nostr-contact-card`,
`@forgesworn/charter` and `@forgesworn/fold-kit`. None provides a direct replacement
for the unpublished keeper or this game's native Rust APIs. Signet Login already
includes its published Signet Verify dependency; loading a second browser SDK is
unnecessary.

## Signet Login workflow

`tools/sites/signet-login-package.mjs` is the shared package contract. The reviewed
`0.19.1` tarball has SHA256
`3269bfc5fe3b389e5dcef8a8e75824248730d54cde895a862b82f12b100550fe`.
Its unmodified browser bundle has SHA256
`f37428d77ff1e49fbda6e6c451bf84ff789f184c1d3d143357992b678c018400`.
Refresh instructions and the offline fallback policy are in
`tools/sites/console/README.md`.

Compatibility dispatches select an exact **published** npm version. A candidate
request without a version, or one whose version is not published, fails. Git
refs/SHAs remain request provenance, not source selectors. The workflow no longer
checks out or builds a Signet repository, follows mutable `latest`, or accepts a
local-built bundle as its reference.

Upstream's inspected release workflow currently requests consumer compatibility
before publishing. That ordering cannot validate an unpublished candidate through
an npm-only consumer. A published prerelease must be made available first, or
upstream release orchestration must be coordinated separately. This repository
does not publish upstream packages or silently fall back to Git source.

The web taster remains anonymous/offline. Console signed proofs still pass the
existing server verification and operator authorisation. These dependency changes
do not enable browser cloud storage, chat, discovery or game hosting.

## Verification

Run the focused Signet consumer tests from the repository root:

```sh
node --test tools/sites/test-signet-login-package.mjs tools/sites/console/test-console-login.mjs
node tools/sites/check-signet-login-bundle.mjs
```

Room tooling has its own offline package API tests and installation instructions
in `tools/room/README.md`. No test opens a real room, contacts a relay, signs in an
operator or publishes a catalogue. `./check.sh` remains the complete regression
gate; focused checks do not replace its engine and browser bundle stages.

Sources: [Signet Login npm metadata](https://registry.npmjs.org/signet-login),
[Signet Login repository](https://github.com/forgesworn/signet-login),
[Forgesworn catalogue](https://forgesworn.dev/),
[canonical catalogue](https://raw.githubusercontent.com/forgesworn/.github/main/forgesworn-repos.json).
