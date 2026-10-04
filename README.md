# Axe'n'Stax

An open-source, self-hostable voxel sandbox. Custom engine, custom client,
custom server — nothing forked or ceiling-limited from an existing project.

**Sovereignty first.** You own your world, your identity, and the server it
runs on. Play solo, self-host with friends, or run a public server — the
game is fully playable, and fully fun, with no money involved at all. The
focus is building, exploring and owning what you make. See `docs/vision/`
for the full thinking behind that ordering.

## Links

| | |
|---|---|
| Home | [axenstax.com](https://axenstax.com) |
| Play in your browser | [play.axenstax.com](https://play.axenstax.com) |
| Download the desktop app | [docs.axenstax.org/download](https://docs.axenstax.org/download) |
| Player guide | [wiki.axenstax.com](https://wiki.axenstax.com) |
| Learn | [learn.axenstax.com](https://learn.axenstax.com) |
| Specs and design docs | [docs.axenstax.org](https://docs.axenstax.org) |
| Project | [axenstax.org](https://axenstax.org) |
| Privacy | [axenstax.com/privacy](https://axenstax.com/privacy) |

The code for every site above lives in `tools/sites/` (see "Building it yourself" below).

## Web taster vs. native

There are two ways to play:

- **Web taster** (`play.axenstax.com`) — an anonymous, local, single-player
  sandbox that runs entirely in your browser (WASM + WebGPU, Chromium-based
  browsers for now). No sign-in, no account, no multiplayer, nothing saved
  or sent anywhere beyond your machine. It's a taste of the game, not the
  full platform.
- **Native** — the full desktop build. Sign in with a self-sovereign
  [Signet](https://mysignet.app) identity, run or join a self-hosted
  world, and keep your own keys. Nothing about the game requires money,
  and the platform never holds your keys.

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

`check.sh` is the single regression gate. Run it locally before you open a PR;
it is not yet run automatically in CI, so green on your machine is the bar for a
change being done.

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

See [CONTRIBUTING.md](CONTRIBUTING.md), including what we won't merge, and the
[Code of Conduct](CODE_OF_CONDUCT.md).

## Security

See [SECURITY.md](SECURITY.md) for how to report a vulnerability.
