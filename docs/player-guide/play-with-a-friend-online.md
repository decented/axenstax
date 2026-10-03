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

The invite lasts **two days**. Press **Host online** again for a fresh one; the
old link stops working straight away.

### "Friends outside your home probably can't reach you"

If you see this, your router isn't letting people in. Turn on **UPnP** in your
router settings and host again. (It's usually under "Advanced", "NAT", or
"Port forwarding".) Your friends on the same wifi can still join either way.

## Joining

**The first time**, use the invite link:

1. Lobby → **Friends & servers** → **Add a friend**.
2. Paste the link, press **Add**. The game calls them straight away.

**After that** they're in your friends list. Press **Join** next to their name
whenever they're hosting — no link needed.

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

Only people in your friends list at **Kin** or **Kith**, plus anyone holding a
live invite link. Everyone else gets nothing at all — the game doesn't even
answer them, so a stranger can't tell whether you're there.

## What the relays see

Setting up a connection uses public Nostr relays for a few messages. They carry
**only** the setup handshake, and it's encrypted: a relay sees two temporary
keys and a timestamp, and never sees who you are, what world you're playing, or
where either of you is. Once you're connected, the game goes **straight** between
your two computers. You can change which relays are used in Settings → Online.
