# Native packaging — installers from the engine binary

Turns the self-contained `axenstax-engine` release binary into **download-and-run**
installers on all three desktop OSes, via [`cargo-packager`](https://github.com/crabnebula-dev/cargo-packager).
This is **Track A** of the native-build strategy
(`docs/architecture/2026-06-06-native-build-distribution-strategy.md`) and bucket 2 of the
solo goal (`docs/goals/native-build-solo.md`) — a **standalone config**, not an edit to
`game/engine/Cargo.toml`.

| OS | Format | Notes |
|----|--------|-------|
| Linux | **AppImage** + `.deb` | No signing gate — `chmod +x` and run. Built **and launch-verified locally**. |
| Windows | NSIS `.exe` | Unsigned → SmartScreen "More info → Run anyway". Built green in CI. |
| macOS | `.dmg` (bundles `.app`) | **Unsigned = Gatekeeper blocks it** (Sequoia removed the easy bypass). Built green in CI; needs A2 notarisation to actually open. |

The engine is fully self-contained — all assets (textures procedural, shaders +
micro-models + registered plans `include_str!`/`include_bytes!`'d) are compiled in, so a
package is just the binary + an icon. Config: `packager.toml` (camelCase keys;
`cargo-packager` uses `deny_unknown_fields`). `name` **must** be set or the tool tries to
`chdir` into the config file to auto-detect a package name and fails (ENOTDIR).

## Build locally (Linux — solo-verifiable)

```bash
tools/packaging/build-local.sh appimage        # or: appimage deb
```

Builds the engine release, stages the binary into `tools/packaging/staging/` (gitignored),
and packages it. To prove the AppImage actually *runs* — not just builds — launch it
headlessly; it renders the lobby to PNGs without a window:

```bash
tools/packaging/staging/axenstax-engine_0.1.0_x86_64.AppImage --shot-lobby /tmp/lobby-shots
ls /tmp/lobby-shots   # lobby-desktop.png / lobby-tablet.png / lobby-phone.png  => it runs
```

(Verified 2026-06-07: AppImage + `.deb` build; the AppImage self-mounts and renders the
full lobby via the GPU.)

## Build all three (CI dry run)

`.github/workflows/native-packages.yml` builds Win/Mac/Linux on hosted runners and uploads
the installers as artifacts. It is a **dry run** — `workflow_dispatch` + a push to the
working branch; **no tag, no GitHub Release, no publish** (the hard line of the goal). To
trigger + watch:

```bash
gh workflow run native-packages.yml --ref native-build-solo   # or just push the branch
gh run watch
gh run download --name axenstax-windows-latest                # fetch the built installers
```

## Boundaries (NOT solo)

- **Confirming the Win/Mac installers *launch* on real desktops** — CI proves they *build*;
  a real Windows / macOS machine has to confirm they *run*.
- **A2 signing + notarisation** — Apple Developer ($99/yr) for macOS (the difference
  between "downloadable" and Gatekeeper-blocked), a Windows cert (or Azure Trusted Signing)
  to remove SmartScreen. Paid accounts → owner. Leave the runbook, stop.
- **Publishing** — itch.io / Flathub / a GitHub Release / a release tag are all
  owner-gated outward pushes.

## Versioning

`version` in `packager.toml` tracks `game/engine/Cargo.toml`'s `[package].version`
(`0.1.0`). Bump both together.
