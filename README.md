# Axe'n'Stax

An open-source, self-hostable voxel sandbox. Custom engine, custom client,
custom server — nothing forked or ceiling-limited from an existing project.

**Sovereignty first.** You own your world, your identity, and the server it
runs on. Play solo, self-host with friends, or run a public server — the
game is fully playable, and fully fun, with Bitcoin switched off. Because
you own the server, you *can* run a peer-to-peer economy and let players
earn real value from play — that's a capability the platform unlocks, not
the headline, and never the reason to play. See `docs/vision/` for the
full thinking behind that ordering.

## Web taster vs. native

There are two ways to play:

- **Web taster** (`play.axenstax.com`) — an anonymous, local, single-player
  sandbox that runs entirely in your browser (WASM + WebGPU, Chromium-based
  browsers for now). No sign-in, no account, no multiplayer, nothing saved
  or sent anywhere beyond your machine. It's a taste of the game, not the
  full platform.
- **Native** — the full desktop build. Sign in with a self-sovereign
  [Signet](https://mysignet.app) identity, run or join a self-hosted
  world, and (on a Bitcoin-enabled server you or someone else runs) take
  part in that server's economy. Non-custodial: the platform never holds
  your funds or your keys.

Both come from the same engine and the same codebase — the web build is a
restricted subset, not a separate product.

## Building it yourself

### Prerequisites (host)

```bash
sudo apt install libudev-dev   # required by gilrs (gamepad support)
# rfd's xdg-portal backend needs xdg-desktop-portal at runtime — standard on
# any desktop Linux, nothing extra to install.
```

You'll also need a stable Rust toolchain (`cargo`) on the host — builds run
on the host, not in a container or VM.

### Build the engine

```bash
cd game/engine && cargo build --release
```

The binary lands at `build/release/axenstax-engine` (relative to your
`CARGO_TARGET_DIR`, or `game/engine/target/release/` if you didn't set one).

### Run the sites

Six small FastAPI apps make up the web side (game, docs, marketing, wiki,
learn, project). Each has its own `tools/sites/<name>/start.sh` (first run
sets up its own venv), or bring them all up together:

```bash
tools/sites/start-all.sh   # tools/sites/stop-all.sh to stop
```

### Release channel

Native builds check for updates over a Nostr-based release channel (see
`tools/release/`). The signing key for that channel is kept manual and
local to the person cutting a release — **it is never added as a GitHub
Actions secret**, so a compromised workflow or a malicious PR can never
forge a release signature.

### Verify your changes

```bash
./check.sh            # clippy (-D warnings), build, full test suite, WASM
                       # bundle build + size gate, docs-site unit tests
./check.sh --smoke     # + a Playwright smoke test against a running site
```

`check.sh` is the single regression gate this project runs in CI and
locally — green there is the bar for a change being done.

## Licence

The code in this repository is [MIT-licensed](LICENSE) — free to use,
modify, self-host, and build on.

**The AxeNStax name and logo are not covered by that licence.** They're
unregistered trademarks used to identify this specific project and the
official builds/services published under it. You're welcome to fork, run,
and modify the code — including for your own server or your own project —
but please don't present a fork, a modified build, or an unrelated service
as "AxeNStax," or reuse the name/logo in a way that could be mistaken for
an official release.

The art Gallery is an optional external world pack (a `.axeworld` loaded
through the normal world import); its artwork is not in this repository and is
**not** MIT-licensed.

This project is not affiliated with, endorsed by, or associated with
Mojang, Microsoft, or Minecraft. Any mention of Minecraft in this codebase
or its docs is nominative reference (compatibility, comparison, or import
tooling), not a claim of affiliation.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## Security

See [SECURITY.md](SECURITY.md) for how to report a vulnerability.
