# Contributing

Thanks for your interest in Axe'n'Stax. This is a small, mostly-solo project,
so the process is deliberately light:

1. **Open an issue first** for anything beyond a small fix — a quick "does
   this fit the direction" check saves everyone time before you invest in a
   PR. For a trivial fix (typo, dead link, obvious bug), a PR on its own is
   fine.
2. **Run `./check.sh` locally before you open a PR.** It's the single
   regression gate (clippy `-D warnings`, build, full test suite, WASM bundle
   build + size gate, the sites' Python tests) and has to pass green. It can be
   run in CI on demand (Actions -> Check -> Run workflow) but does not run
   automatically on pushes or PRs yet, so nobody else will run it for you.
3. **Keep PRs focused.** One change, one PR — easier to review, easier to
   revert if something's wrong.
4. **Follow the existing code style and patterns** rather than introducing
   a new one. If you're not sure why something's done a particular way,
   ask in the issue rather than guessing.
5. By submitting a PR, you agree your contribution is licensed under this
   repository's [MIT licence](LICENSE).

## What we won't merge

Axe'n'Stax is a kids-adjacent project, and its compliance posture rests on one
principle: the laws that matter here regulate the *operated service* and whoever
controls access to it, not neutral, self-hostable software. We ship software;
we never become the operator. So a PR will be declined, however well made, if it
crosses any of these four lines:

1. **No AxeNStax-operated public directory** that discovers or lists player-run
   servers, worlds or groups. Discovery stays LAN-local, opt-in self-published
   Nostr announce (never automatic), or direct address. Any browser or
   matchmaking feature must be opt-in, default-unlisted and child-safe.
2. **No AxeNStax-operated game servers, or relays that carry a group's
   traffic.** Worlds are self-hosted and connect directly; multiplayer does not
   run over Nostr, and relays carry setup only, never game traffic, presence or
   in-game chat. No hosted game or cloud fleet for other people's groups.
3. **No central collection of children's data.** The web build stays anonymous
   and local (no login, cookies, analytics or age data). Cloud storage stays
   bring-your-own-key and ciphertext-only. Feedback stays burner-key, end-to-end
   encrypted and owner-local. Never collect age as raw data (at most a yes/no
   from a third-party identity provider, never a date of birth), and send no
   child's personal information to any AxeNStax-operated server.
4. **No positioning as a social network.** Never describe or market the
   product as a social network, kids' social media or chat platform. "Play with
   friends" means building together. Marketing claims must never run ahead of
   what is built (do not advertise safety or parental controls that do not
   ship).

**Facilitate, never induce.** Neutral, documented, self-hostable software is
fine. We will not merge a "dev mode", flag or documentation whose real purpose
is to show people how to switch safety features off. Sensitive features (such as
communication) sit behind a real capability or age boundary from a third-party
identity provider, not a disclaimer.

If you are unsure whether an idea crosses a line, open an issue and ask before
you write the code.

## Children and personal details

Never post a child's personal details, photos, real names or location in an
issue, PR, comment or screenshot. Redact them before you post, and keep test
data fake.

## Conduct

Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

For anything security-related, please see [SECURITY.md](SECURITY.md)
instead of opening a public issue or PR.
