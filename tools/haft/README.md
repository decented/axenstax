# Haft McHafty — the owner's KithMoot agent

Haft is the owner's agent in the AxeNStax world's chat room: an AxeNStax
character who answers when addressed, catches things worth passing on, and
is honest about being an agent and about whose it is. Design and the
regulatory reasoning behind "an agent must be owned, by default" are in
`docs/foundations/2026-09-05-world-chat.md` §4.3; Haft's character brief is
`tools/haft/haft-persona.md`.

This page is the operator's page: the exact steps from nothing to Haft
sitting in a room, what only the owner can do, and what is not built yet.

## What's already true on this machine

- KithMoot lives somewhere on this machine, built (`dist/` exists) —
  `install.sh` defaults to `$HOME/kithmoot` and takes `KITHMOOT_DIR=` to
  override if yours is elsewhere.
- Haft's identity key exists: `~/.kithmoot/haft.key` (mode 0600).
- Haft's memory directory exists, empty: `~/.kithmoot/haft/`.

None of the three were created by `install.sh` — it checks for them and
refuses to proceed if the key is missing, on purpose (see its comments).

## What is honestly NOT done yet

- **No room exists.** A room needs a keeper (`kithmoot-agent create`)
  holding it open. The engine's own room plug — `WorldRoom` /
  `KithMootKeeper`, `/room` commands — is Phase 6 of
  `docs/foundations/2026-09-05-world-chat.md` and has not landed in
  `game/engine/` as of this writing. Until it does, the only way to get a
  room link at all is to run a keeper by hand (§3 below).
- **No ownership proof exists.** It has to be signed by the *owner's* key.
  This session cannot mint it and has not gone looking for the owner's key
  file — see §2.
- **The systemd unit is installed but not started.** `install.sh` stops
  there deliberately: starting it before the above two exist just gets you
  a unit that restarts forever logging "join needs a link".

## 1. Install the unit

```bash
tools/haft/install.sh
# or, if kithmoot isn't at $HOME/kithmoot on this machine:
KITHMOOT_DIR=/path/to/your/kithmoot/checkout tools/haft/install.sh
```

Idempotent. Verifies the kithmoot build and the identity key, writes
`~/.config/axenstax/haft.env` from `haft.env.example` (only if that file
doesn't already exist — safe to re-run after editing it), installs
`~/.config/systemd/user/haft.service`, and runs
`systemctl --user daemon-reload`. It does not start anything.

The rest of this page uses `$KITHMOOT_DIR` for the checkout path — set it
in your shell to whatever you gave (or would give) `install.sh` above:

```bash
export KITHMOOT_DIR=$HOME/kithmoot   # or your override, same value
```

## 2. Find Haft's own npub

There's no `kithmoot-agent whoami`. But the identity check runs, and logs
the resolved npub to stderr, before the process tries to do anything with a
room — so you can read it off without a real link. From your kithmoot
checkout (`$KITHMOOT_DIR` — wherever `install.sh` found or was told it
lives):

```bash
cd "$KITHMOOT_DIR"
node bin/kithmoot-agent.mjs join placeholder \
  --identity ~/.kithmoot/haft.key --name Haft --brain none
```

This will print a line like:

```
[kithmoot-agent] identity npub1...
```

and then fail on the placeholder link (`kithmoot-agent: join URL has no
fragment`, or similar) — that failure is expected and fine; you only wanted
the identity line. Because `~/.kithmoot/haft.key` already exists, this does
**not** mint anything or touch the key. Save the npub — you need it twice:

- as `--agent <npub>` in the attest command below, and
- as `--expect-pubkey` (`KITHMOOT_EXPECT_PUBKEY` in `haft.env`), so that if
  this key file is ever moved, deleted, or replaced, Haft refuses to start
  as a silently-different identity rather than running on as an agent that
  *looks* like Haft and isn't.

## 3. Get a room and a link

**Only if no keeper is running yet for this world.** If the engine's own
room plug (Phase 6) has shipped and a world already has `/room invite`
working, skip this — use that link instead.

Otherwise, stand up a keeper by hand, the same way
`forgesworn/kithmoot/deploy/keeper@.service` does it in production, just
run directly instead of as a unit for now:

```bash
cd "$KITHMOOT_DIR"
node bin/kithmoot-agent.mjs create \
  --base <wherever KithMoot's own web app is served, e.g. https://kithmoot.forgesworn.dev/j/> \
  --name Keeper --room-name <world name> \
  --relays <family relay 1>,<family relay 2> \
  --state ~/.kithmoot/keeper-<world>/room.json \
  --identity ~/.kithmoot/keeper-<world>/identity.key
```

It prints the room link once and writes it beside the state file at
`~/.kithmoot/keeper-<world>/room.json.link` (owner-readable only) on every
restart after. **Not trotters** for `--relays` —
`docs/foundations/2026-09-05-world-chat.md` §4.4 explains why at length; the
short version is trotters is AxeNStax's own relay and must never carry a
family's conversation. Use the
family's own two or three relays.

Keep that process running (or make it a unit later, following
`deploy/keeper@.service` as the template) for as long as the room should
stay open.

## Two ways to run Haft — pick before you read further

**Session-driven (the owner's preference, and no API key anywhere).**
`tools/haft/session-join.sh '<room link>'` puts Haft in a room on KithMoot's
`stdio` brain and lets a Claude Code session be his brain over a pair of pipes.
Nothing is billed to a model key, nothing runs when nobody is home, and the
model in the room is the same one you are already talking to. He answers at the
speed the session notices, and goes quiet when it ends.

**Daemon (this document's original route).** `haft.service` plus an
`ANTHROPIC_API_KEY` in `haft.env` runs him always-on, on his own key. Right for a
standing room nobody is watching; wrong if you would rather not have a key
sitting on disk.

Everything below — the identity, the npub, the ownership proof — is needed by
**both**. Only the last step differs.

## 3b. The keeper needs one of these too

Worth knowing before you get to step 4, because it is not obvious: **you will
mint two ownership proofs, not one.**

A keeper sits in the room marked as an agent. In a room that requires agents to
be owned by a member — which is the only kind this project will attach to — an
agent that cannot show whose it is gets refused, and that includes the room's own
keeper. "It opened the room" is not attribution.

So the keeper's key gets exactly the same treatment as Haft's: start it once to
mint its identity and read its npub off stderr, attest that npub with your own
key, and pass the result back with `--owner-proof`. `tools/room/create-room.mjs`
takes that flag, and warns you up front if you start without it rather than
failing later with a stack trace.

Full walkthrough for the room side: `docs/operators/world-chat.md`.

## 4. The owner mints the ownership proof

**This step is the owner's, not this session's, not this script's.** Run it
yourself, as yourself, with your own key file:

```bash
cd "$KITHMOOT_DIR"
node bin/kithmoot-agent.mjs attest \
  --agent <Haft's npub, from step 2> \
  --identity <YOUR OWN key file — not Haft's, not this session's> \
  --label "Haft McHafty" \
  --expires 90d
```

This prints JSON to stdout — an `AgentOwnership` record signed by your key.
It is not published anywhere by this command; it's just a file. Save it:

```bash
node bin/kithmoot-agent.mjs attest ... > ~/.config/axenstax/haft-owner-proof.json
chmod 600 ~/.config/axenstax/haft-owner-proof.json
```

(That's the default path `haft.env.example` already points
`KITHMOOT_OWNER_PROOF` at — keep it there and there's nothing else to
change.) `--expires 90d` means re-run this in 90 days; a proof with no
`--expires` never lapses, which is more convenient and a worse default for
something a room full of people rely on to say whose Haft is.

## 5. Fill in the env file

Edit `~/.config/axenstax/haft.env`:

```bash
KITHMOOT_LINK=<the room link from step 3>
KITHMOOT_EXPECT_PUBKEY=<Haft's npub, from step 2>
KITHMOOT_OWNER_PROOF=$HOME/.config/axenstax/haft-owner-proof.json   # already the default
KITHMOOT_RELAYS=<the same family relays used in step 3>
ANTHROPIC_API_KEY=<a real key, if you want Haft answering on the API tonight>
```

Every line in the file is commented with what it is and whether it's
required — see `tools/haft/haft.env.example`.

## 6. Start it

```bash
systemctl --user enable --now haft
```

`enable` makes it come back after you log back in (it's a user unit — it
does *not* survive past logout unless lingering is turned on with
`loginctl enable-linger $USER`, and it does not run at all while the
laptop is off, which is the intended behaviour: Haft sleeps when the
laptop does).

## 7. Check it's running

```bash
systemctl --user status haft
journalctl --user -u haft -f
```

The first successful join logs a line like `joined room <id> as
<participant>, answering the link` (from KithMoot's own `RoomAgent.join`
logging). If instead you see `join needs a link` on repeat, `KITHMOOT_LINK`
is still blank in `haft.env`. If you see `refusing to run: expected
<npub-a> but resolved <npub-b>`, `KITHMOOT_IDENTITY` or
`KITHMOOT_EXPECT_PUBKEY` disagree with each other or with the actual key
file — that's the check working, not a bug.

## 8. Stop it

```bash
systemctl --user stop haft        # stop now, unit still enabled
systemctl --user disable --now haft   # stop and don't restart on next login
```

Stopping Haft does not close the room or remove it from the roster
permanently — it just leaves. The keeper (§3) is what keeps the room open;
stopping that is a separate, and much bigger, decision.

## Files in this directory

| File | What it is |
|---|---|
| `haft-persona.md` | The character brief passed as `--persona` / `KITHMOOT_PERSONA`. |
| `haft.env.example` | Template for `~/.config/axenstax/haft.env`, one comment per line. |
| `haft.service` | The user-level systemd unit. |
| `install.sh` | Idempotent installer — see its header. Does not start the service. |
| `README.md` | This page. |
