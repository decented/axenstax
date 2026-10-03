# AxeNStax release tooling

Publishes AxeNStax AppImage releases as kind-**30063** Nostr events
(`d = axenstax-appimage`), signed by a dedicated release key, pointing at
AppImage copies mirrored on Blossom servers. Lifted from Vitark Train's
`tools/release/` (itself ported from Fathom's
`client/scripts/release/`) — see `release-helpers.mjs`'s header comment for
the itemised list of what differs here and why.

## Why this exists

The update path players are actually on today reads
`https://docs.axenstax.org/download/latest.json`, served by one box. That box
is currently three versions behind because publishing to it goes over SSH to
an sshd that is unresponsive — a single host, single point of failure. This
channel routes around that host entirely: three independent relays plus
whichever Blossom mirrors verify, none of them ours to keep online.

**Which relays.** The app reads this channel from the player's own "Your
relays" list, whose default is the public set in
`server_resolve::PUBLIC_DEFAULT_RELAYS` (`relay.damus.io`, `nos.lol`,
`relay.primal.net`). So the release is published there first. A player who
removes every public relay still learns of updates through the HTTPS check
(notify-only); there is no hidden fallback relay. `relay.trotters.cc` is
kept as an extra publish target only — it carries the older release history
the monotonicity gate reads, and publishing our own public release notes on
our own relay is harmless — but it is never a default the app reads.

## The channel

- Kind `30063`, `d = axenstax-appimage`.
- Tags, in order:
  - `d` — `axenstax-appimage`
  - `version` — plain numeric dotted version (`0.2.21`), read from the
    artifact's filename and cross-checked against
    `game/engine/Cargo.toml`
  - `x` — sha256 of the AppImage bytes
  - `size` — bytes, decimal string
  - `url` — one **repeatable** tag per verified Blossom mirror
- No `version_code` — there's no Android build number for a Linux binary.
- No `cert` tag — there is no AppImage signing certificate yet. **Add it
  back** (in `buildReleaseEvent` and the tag order above) when signing
  lands; until then the integrity guarantee is entirely the `x` sha256 tag
  plus the pinned release pubkey's signature over the whole event.
- `content` is free-text release notes (`--notes` or `--notes-file`).
- Relays: `wss://relay.damus.io`, `wss://nos.lol`, `wss://relay.primal.net`
  (the app's defaults), plus `wss://relay.trotters.cc` as an extra target.
  Check the per-relay `relay ok` / `relay FAIL` lines after a publish: a
  2026-10-02 read-only probe found the current release event on none of the
  three public relays.
- Blossom mirrors (default, override with `AXENSTAX_BLOSSOM_SERVERS`):
  `https://blossom.primal.net`, `https://nostr.download`.

## Setup

```bash
cd tools/release
npm install
```

## Key ceremony

```bash
node tools/release/new-release-key.mjs
```

Writes `~/.axenstax-release/release-key.hex` at `0600` inside a `0700`
directory, and prints the resulting pubkey hex and npub. **Refuses to
overwrite an existing key file** — rotating it is a deliberate, disruptive
act (every installed build that trusts the old pubkey would need to be told
about the new one some other way), never an accident. The secret never
leaves that file: don't print it, commit it, or paste it into a chat.

This is a **dedicated release key**, deliberately separate from the
AxeNStax project identity npub (kept outside this repo). See
`new-release-key.mjs`'s header comment for why — short version: different
blast radius (this key only ever signs release announcements + Blossom
upload auth) and it may live on a different machine than the identity key.

**This ceremony has NOT been run for real yet.** `RELEASE_PUBKEY_HEX` in
`release-helpers.mjs` is currently `''`. The first time it IS run for real:

1. Run the command above.
2. Paste the printed `RELEASE_PUBKEY_HEX` value into the constant of the
   same name in `release-helpers.mjs`.
3. Commit that change.

Until that pin exists, `publish-release.mjs` and `query-latest.mjs` cannot
tell a real release from anyone else's kind-30063 event with the same `d`
tag — they print a loud warning and carry on unverified. This is a
deliberate bootstrapping gap, not an oversight: there is no way to pin a
pubkey before one has been generated. **Do not skip step 2** — publishing a
"real" first release with the pin still empty would announce a channel
nothing can ever authenticate.

## Cutting a release

1. Build the AppImage the normal way (`tools/packaging/`) so its filename
   follows the `axenstax-engine_<version>_x86_64.AppImage` convention —
   that's what `parseAppImageVersion` in `release-helpers.mjs` expects, and
   it's the same convention `tools/sites/docs/versioning.py` already relies
   on for `/download/latest.json`.
2. Check what's already live:
   ```bash
   node tools/release/query-latest.mjs
   ```
   Exits `0` printing the highest verified live version (or `none`), or `3`
   if no relay answered — treat exit `3` as "couldn't check", never as
   "nothing is live," and don't publish past it without `--force`.
3. Publish:
   ```bash
   node tools/release/publish-release.mjs \
     --artifact <path-to-AppImage> \
     [--notes "…" | --notes-file <path>] \
     [--dry-run] [--force]
   ```
   There is no `--version` flag: the version comes from the artifact's own
   filename, and is refused if it disagrees with
   `game/engine/Cargo.toml`'s `version` — the artifact and the source tree
   that built it have to agree before anything gets announced.

   `publish-release.mjs` then:
   - Runs its **own** monotonicity check against the release relays (the
     same query `query-latest.mjs` does) and refuses to publish a version
     that is not strictly newer than what's live, unless `--force` is
     passed. This is the one deliberate structural difference from
     Vitark: there, that check is a separate manual step the operator runs
     first; here it's built into the publish path itself, because a
     downgrade on this channel silently strands every player whose
     updater trusts it, and a step that depends on the operator
     remembering to run it first is a step that eventually gets skipped.
   - Uploads the AppImage to every Blossom mirror with BUD-02 auth,
     re-fetches each mirror with `redirect: 'manual'`, and checks the
     sha256 of the bytes actually served at the mirror's **canonical**
     root address (`https://host/<sha>[.ext]`) — never a CDN redirect
     target, which can vanish out from under a client with no fallback
     (see the "kintrinsic" note in `publish-release.mjs`'s
     `verifyBlossom`).
   - Refuses to announce unless at least one mirror verified that way.
   - Signs and publishes the kind-30063 event to every release relay, and
     succeeds if **any** relay accepts it.

   `--dry-run` signs and prints the event but touches **no network at
   all** — no live-version query, no upload, no publish. It still needs a
   real key file to sign with (see below). Use it to sanity-check the
   event shape before committing to a real run.
   `--force` overrides the monotonicity refusal only — it does not bypass
   the filename/Cargo.toml version check, and it does not bypass the
   Blossom-upload-must-verify check.

## Verifying it worked

```bash
node tools/release/query-latest.mjs
```

Prints the version, sha256, size, and every verified mirror URL of the
live release — this is what a human runs to check what players are
actually being offered, independent of any one web host.

## Env vars

| Var | Default | Purpose |
|---|---|---|
| `AXENSTAX_RELEASE_KEY_FILE` | `~/.axenstax-release/release-key.hex` | Where the release secret lives |
| `AXENSTAX_BLOSSOM_SERVERS` | `https://blossom.primal.net,https://nostr.download` | Comma-separated Blossom mirror list |

## Failure modes, named up front

- **No relay answers the monotonicity check.** `publish-release.mjs`
  refuses to publish (exit 1) unless `--force` is passed — it will never
  silently treat "couldn't check" as "safe to publish." If this happens
  when all three relays are actually healthy, it's a network problem on
  the machine running the script, not the channel.
- **A Blossom mirror uploads fine but redirects on fetch.** That mirror
  contributes no `url` tag (see `verifyBlossom`) — the release still
  publishes on whatever mirrors DID verify directly. If none did, the
  whole publish refuses rather than announcing a release nothing can
  actually download.
- **`RELEASE_PUBKEY_HEX` is still unpinned when someone runs
  `query-latest.mjs`.** It will always print `none`, even after a real
  publish, because nothing can be verified as authentic without the pin.
  This is not a bug in the query — it's the bootstrapping gap described
  above. Pin the key first.
- **The artifact's filename doesn't match `axenstax-engine_<version>_x86_64.AppImage`.**
  `parseAppImageVersion` throws and nothing is published. This is
  deliberate — a renamed or hand-copied file is exactly the case where
  trusting the filename would be dangerous.
- **Filename version and `Cargo.toml` version disagree.** Refused,
  unconditionally — there's no `--force` for this one, because it's not a
  "we know better" situation, it's "one of these two is simply wrong."
- **Every relay refuses the signed event on publish.** Exit 1, nothing
  announced. Re-run — this has historically been transient relay load, not
  a structural problem with the event.

## Testing

```bash
cd tools/release && npm install && node --test
```

Covers `release-helpers.mjs`'s pure builders (tag order and shape,
malformed-input rejection, canonical-URL matching, filename version
parsing, numeric version comparison, and event-authenticity filtering).
None of it touches the network.
