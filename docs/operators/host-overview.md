# Host your own Axe'n'Stax server

You don't need our servers to play together. Axe'n'Stax is **self-hostable** — you
can run your own server on a spare computer, a little home server (NAS), or a cheap
cloud box, and invite whoever you like. **Your world, your rules.**

This is the short version of what it is and why you'd want it. When you're ready to
do it, the **[Run Your Own Server](dedicated-server.md)** page has the exact steps,
and the **[Operator Console](operator-console.md)** page covers managing it.

---

## Why run your own?

- **Play with friends and family** — one shared world that everyone joins, from the
  browser *or* the app, no accounts required.
- **You own it.** The world saves live on *your* machine. Nothing phones home, and
  no one can take it away or change the rules on you.
- **You set the rules.** Who can join (open, invite-only, or sign-in-required),
  the game mode, the name, the privacy posture — all yours to decide.
- **It's a great thing to learn.** Running your own server is real, hands-on
  ownership — the same idea as running your own website or your own wallet.

> **Just playing with a friend in another house?** (New — if it doesn't punch
> through on your router, a dedicated server below is the fallback.) You don't
> need any of this —
> no Docker, no box that has to stay on. In the desktop app, pick **Host online**
> and share your Signet persona npub (or a one-off invite link) with your friend;
> they add you as a contact or open the link, and the two apps find each other and
> connect directly. Nostr relays only carry that one-time connection setup, never
> any game traffic. Admission is automatic for a contact at Kin or Kith, or for
> whoever is holding your live invite — everyone else gets silence, not an error.
> See [Play with a friend online](../player-guide/play-with-a-friend-online.md).
> Run a dedicated server (below) when you want a world that stays up whether or
> not you're playing, or that more than a couple of friends can drop into.

---

## What you need

- **A computer that stays on** while people are playing — a spare laptop/PC, a NAS
  (Synology, QNAP, a Raspberry Pi), or a small cloud server (VPS).
- **[Docker](https://docs.docker.com/get-docker/)** installed on it. That's the
  only thing you install — Docker fetches everything else.
- **Players on the same network** (home/LAN) can join straight away. To let people
  join **over the internet**, you'll either forward a port on your router or use a
  cloud box with a domain name — the [Run Your Own Server](dedicated-server.md) page
  walks through both.

That's it. No game files to compile, no command-line wrangling beyond a couple of
copy-paste commands.

---

## How it works, in one picture

One small Docker container runs **two things behind one secure web address**:

- the **game server** — the real, authoritative world (blocks, mobs, carts, economy,
  saves), running without a screen;
- a **web front** that serves the browser version of the game *and* the connection
  for the app, so a browser trusts the security certificate once and reuses it.

Players just open `https://<your-box>` in Chrome/Edge, or pick **Join Game** in the
app and point it at your box. Both land in the same world.

---

## Getting started

1. **[Run Your Own Server](dedicated-server.md)** — install + start it, and get
   people joining (home network, then over the internet).
2. **[Operator Console](operator-console.md)** — a small web page where you sign in
   with your own identity to manage who can join, the server's name and limits, and
   privacy — no command line.

---

## Good to know (alpha honesty)

This is early. Today a self-hosted server is a **single shared world**, players join
as **guests** by default, and a couple of multiplayer pieces are still being wired up
(listed at the bottom of the [Run Your Own Server](dedicated-server.md) page). It's
real and it works — just know it's a moving target while we're in alpha.
