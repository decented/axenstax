# Contributing

Thanks for your interest in Axe'n'Stax. This is a small, mostly-solo project,
so the process is deliberately light:

1. **Open an issue first** for anything beyond a small fix — a quick "does
   this fit the direction" check saves everyone time before you invest in a
   PR. For a trivial fix (typo, dead link, obvious bug), a PR on its own is
   fine.
2. **Run `./check.sh` before you open a PR.** It's the single regression
   gate (clippy `-D warnings`, build, full test suite, WASM bundle build +
   size gate) and has to pass green.
3. **Keep PRs focused.** One change, one PR — easier to review, easier to
   revert if something's wrong.
4. **Follow the existing code style and patterns** rather than introducing
   a new one. If you're not sure why something's done a particular way,
   ask in the issue rather than guessing.
5. By submitting a PR, you agree your contribution is licensed under this
   repository's [MIT licence](LICENSE).

For anything security-related, please see [SECURITY.md](SECURITY.md)
instead of opening a public issue or PR.
