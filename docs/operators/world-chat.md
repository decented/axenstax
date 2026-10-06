# World chat — running it

For somebody hosting a world. What chat does, what you control, and the things
that will surprise you if nobody says them first.

Design and reasoning: `docs/foundations/2026-09-05-world-chat.md`.

---

## The short version

- **Native only.** The web build has no chat at all, and a `check.sh` gate fails
  the build if any of it leaks in. The web taster is anonymous and local; chat
  would make it a service.
- **Signed-in players only.** No verified key, no chat.
- **Charter sets a child's ceiling. You can only tighten it, never loosen it.**
- **Relays for a room are yours.** There is no default, on purpose.

---

## Who can talk to whom

Every player has a level:

| Level | Hears | Speaks to |
|---|---|---|
| `blocked` | nobody; the chat UI is hidden | nobody |
| `approved` | kin, kith, **and anyone they ken** | kin and kith |
| `anyone` | everybody in the world | everybody in the world |

The three relationships come from the player's own contact list:

- **kin** — family. Two-way.
- **kith** — mutually verified. Two-way.
- **ken** — you have recognised somebody who has not recognised you back.
  **Hear-only.** You can hear them; you cannot speak to them.

That last one is the point of the whole design. A child can follow a host or a
well-known builder without handing that person a way to talk to them.

A line is delivered only if **both** sides agree: the speaker is allowed to speak
to that listener, and the listener is allowed to hear that speaker. So an adult
on `anyone` still does not reach a child on `approved` who has not ken'd them.

All of this is decided on the server, once per recipient. Nothing is filtered on
the client, because a client that filters is a client that can be patched not to.

---

## What you control

```
--chat-level blocked|approved|anyone      (or AXENSTAX_CHAT_LEVEL)
```

This is a **ceiling on your world**, applied to everybody in it. The effective
level for each player is `min(their Charter level, your world level)`.

You can only tighten. Setting `anyone` does not grant anything — it means you
have declined to tighten. If a guardian has set a child to `approved`, your
world cannot raise them to `anyone`, and it should not be able to.

A typo in the flag is a startup error, not a silently permissive world.

---

## Sign-in, and why chat needs it

**A player with no verified key gets no chat** — the UI is hidden and their
messages are refused.

This is deliberate, and it is not about tidiness. A tier system without identity
is meaningless: an anonymous player is a stranger to everybody. More importantly,
if anonymous players could chat, a child would escape their guardian's ceiling by
simply not signing in. The gate has to be the key.

**What this means for your world:** if you admit unauthenticated players (a
dedicated server started with `--allow-guests` / `AXENSTAX_ALLOW_GUESTS=1`; sign-in
is required by default), those players have no chat. Everyone who signed in still
does. If it matters to you that every player is governed by their guardian's
settings, keep sign-in required.

Hosting a world yourself? Your own slot takes its identity from your sign-in on
that machine, so you can chat in a world you host. **Split-screen players 2 and
up share your sign-in, have no identity of their own, and get no chat** — they
are next to you and can speak out loud.

---

## Rooms — reaching somebody who is not in the game

A room lets somebody on a phone or in a browser take part in a world's chat. It
runs over [KithMoot](https://kithmoot.com).

### Relays are yours

Supply two or three `wss://` relays your family or group already uses.

**There is no default, and that is on purpose.** KithMoot's own default relay
list begins with `relay.trotters.cc`, which is AxeNStax's own relay (no longer a
default anywhere in the AxeNStax app itself). It must never carry a group's
conversation, so the engine refuses an empty relay list rather than falling back,
and strips that host out of any list you give it, with a reason.

```
AXENSTAX_ROOM_RELAYS=wss://your.relay,wss://another.relay
```

### Opening a room

```bash
AXENSTAX_KITHMOOT_DIR=<your kithmoot checkout> \
node tools/room/create-room.mjs \
  --name "Keeper" --room-name "Home" \
  --relays wss://your.relay,wss://another.relay \
  --state $HOME/.config/axenstax/room.json
```

Keep it running — it is what answers the link. The link is printed once and
written to `<state>.link`, readable only by you. A link is a capability; treat it
like one.

**Why a script rather than `kithmoot-agent create`:** the engine refuses any room
whose link does not require agents to be owned by a member, and KithMoot's CLI
has no flag to set that. Its library does, so this script is their own create
call plus the one option the CLI omits. It goes away when the flag lands
upstream.

### You will need to attest the keeper

A keeper sits in the room marked as an agent, and an owned-by-members room
refuses an agent that cannot show whose it is — **including the room's own
keeper**. That is the rule working. "It opened the room" is not attribution.

So, once:

1. Start the keeper once. It mints its identity and prints its npub.
2. Attest it with **your own** key:

```bash
node bin/kithmoot-agent.mjs attest \
  --agent <keeper npub> --identity <your own key file> \
  --label "world-chat keeper" > keeper-owner.json
```

3. Restart with `--owner-proof keeper-owner.json`.

The same applies to any agent you put in the room — see `tools/haft/README.md`.

### What the room does and does not see

A line is mirrored out to the room only if the speaker would have been allowed to
speak to at least one of its members.

In practice: **a child on `approved` is mirrored to a family room whose members
are kin. They are not mirrored to a room of your acquaintances they have never
met.** The room is a member of the conversation, not a way around it.

Lines coming back from the room reach each player under the same hearing rule
that governs everyone else.

### `/room` in game

```
/room                  status — attached, the link, members, relays
/room join <link>      attach to a room
/room invite           print the link to copy
/room leave            detach
```

Rotation is not available from the game: rotating a room is a keeper operation
and the game joins as an ordinary member.

---

## Rate limits and message size

30 lines per minute per player; longer lines than 256 characters are refused.

Refused lines are **rejected, not trimmed** — a shortened sentence is a changed
sentence, and the sender is told which rule they broke rather than having their
words quietly altered. Someone going too fast is told once, not thirty times.

The 30/minute figure matches KithMoot's own limit, so a room and a world cannot
flood each other.

---

## Not built yet

Stated plainly so you do not plan around it:

- **Guardian copy** — sending a child's own conversation to their guardian's key
  is specced and its payload handling is written and tested, but **the send path
  and the in-game indicator are not wired**. It does nothing today.
- **Proximity voice** — not started, by design. It will get its own guardian
  permission; permitting typing is not permitting an open microphone.
- **Charter comms clause** — Charter has no comms capability yet, so a child's
  ceiling currently comes from a guardian-signed local file
  (`~/.config/axenstax/guardian-policy.json`), signature-checked on load. An
  unsigned or altered file is ignored entirely. The upstream ask is filed.
