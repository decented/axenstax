# Security Policy

## Reporting a vulnerability

Please **don't** open a public GitHub issue for a security problem.

Report it privately through this repository's
[private vulnerability reporting](../../security/advisories/new). We don't have
an email address for security reports, so please use that channel.

## Scope

This covers the AxeNStax engine (native + WASM/web builds), the dedicated
server and its operator console, the six website apps under `tools/sites/`,
the Nostr release/update channel, and this repository's
CI (GitHub Actions workflows and their secrets).

## What we especially want to hear about

In rough priority order:

1. Anything touching a child's identity or data — the web taster, Signet
   sign-in, or any path that could expose or misuse
   personal data from a minor.
2. A way to loosen the local safety controls: the tightening-only comms policy
   file (which may only lower a player's chat ceiling, never raise it) or the
   `charter_allows_sats` flag that gates the Bitcoin layer.
3. A forged join event, operator credential, or release/update signature —
   anything that lets someone impersonate a player, an operator, or an
   official AxeNStax release.
4. Remote code execution on a player's machine (native client, dedicated
   server, or a website).

Lower-severity issues (denial of service, non-sensitive info disclosure,
etc.) are still welcome, just not as urgent.

## Supported versions

Only the latest published release gets fixes. If you are on an older build,
please update before reporting, or say which version you tested.

## What to expect

This is a small, mostly one-person project — reports are handled on a
best-effort basis. You can expect an acknowledgement within about 7 days.
We aim to disclose within 90 days of a confirmed report, or sooner once a
fix has shipped, whichever is later; we'll coordinate timing with you if
there's a reason to wait. There's no bug bounty. If you'd like credit, tell
us the name (or npub) to credit you under and we'll include it in the
release notes or changelog.
