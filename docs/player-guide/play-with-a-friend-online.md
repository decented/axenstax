# Play with a friend online

You can host a world on your own computer and let a friend in another house
join it. Your computer is the server. Nothing runs on ours.

> This is a **brand-new** feature. It's built and tested, but every router is
> different, and it hasn't yet been tried across two real houses by the team.
> If it doesn't work for you and your friend, tell Staxolottle what happened —
> that's exactly the feedback that gets it fixed.

## What you both need

- The **desktop app** (this doesn't work in the browser).
- To be **signed in** with your Signet persona, on both computers.
- To be **online at the same time** — you're calling each other, so you both
  have to pick up.

## Hosting

1. In the lobby, find your world and press **Host online**.
2. The game copies an **invite link** to your clipboard. Send it to your friend
   however you normally talk to them.
3. That's it — you're hosting. Type `/online` in the game any time to see the
   link again, or `/online copy` to copy it.

An invite lets in **the first person who uses it**, into this world only. Send it
to the person you mean, and only them: whoever uses it first owns it. They can come
back with it if they drop out, and once they have used it, nobody else can get in with
it. The link works for two days. Someone it has let in can stay until you stop hosting
or block them. It doesn't make them your friend, and it doesn't work for your other
worlds.

If you hand the world to someone else to host, that host starts with no record of who
used a link, so on their machine the first person to use it is the one it lets in.

Want to invite someone else? Type `/online copy` for a new link once the first one
has been used. Nobody new can use the old link, but the person it let in still can.

### "Friends outside your home probably can't reach you"

If you see this, your router isn't letting people in. Turn on **UPnP** in your
router settings and host again. (It's usually under "Advanced", "NAT", or
"Port forwarding".) Your friends on the same wifi can still join either way.

## Joining

**With an invite link:**

1. Lobby → **Friends & servers** → **Add a friend**.
2. Paste the link, press **Add**. The game calls them straight away.

Keep the link: it gets you back into that world if you drop out, until it expires.
Using one doesn't add anybody to your friends list.

**With a friend in your list:** press **Join** next to their name whenever they're
hosting. If the game says they haven't played online with you yet, join once with an
invite link from them. After that, **Join** works with no link.

## Your address

At the top of the Friends column is **your address** — an `npub`, with a QR code
and a **Copy** button. Give it to a friend so they can invite you. It's just an
address; nobody can do anything with it except invite you.

## If it doesn't work

| What you see | What to do |
|---|---|
| *"… didn't answer. Are they online with the world open?"* | They need the game open with the world hosted. Ask them. |
| *"Couldn't reach …'s world."* | Their router needs UPnP turned on. Ask them to check Settings → Online. |
| *"You're on different versions."* | One of you needs to update the game. |
| *"…'s world is full."* | Wait for a space. |

## Who can join

Only people in your friends list at **Kin** or **Kith**, plus the one person each
invite link has let in. Everyone else gets nothing at all — the game doesn't even
answer them, so a stranger can't tell whether you're there.

## What the relays see

Setting up a connection uses public Nostr relays for a few messages. They carry
**only** the setup handshake. The contents of those messages (the addresses your
two computers use to reach each other) are encrypted, so a relay can't read
them. A relay can still see your **IP address**, which keys each message is
addressed to (an install key your game makes for this, not your Signet public
key), and **when** you check for messages. It never sees what world you're
playing.

Once you're connected, the game goes **straight** between your two computers,
which means **you and your friend can see each other's IP addresses**. Only
play with people you trust. You can change which relays are used in
Settings → Online.
