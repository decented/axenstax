# Bitcoin & Sats

You'll see the word "sats" mentioned around the game — in quest text, Vendor
Blocks, Plaques, the Furnace. This page is the honest, short answer to what
that means today.

## What a sat is

A **sat** (satoshi) is the smallest unit of **Bitcoin**. That's it — a unit
name, like "penny" or "cent."

## Switched off everywhere, right now

Bitcoin is a designed-but-not-built layer. Today, on **every** world —
sandbox or server, whatever a server operator sets — **no sats show up, no
real money changes hands, and there's no wallet.** The plumbing that would
one day turn a hash into a real Lightning payment isn't wired up yet, so
nothing anywhere converts to anything spendable.

You don't need Bitcoin switched on to enjoy the game. Items, reputation,
quests, villages, Knights — all of it works fully with sats off. Barter mode
covers trading without ever mentioning a number.

## No controls to set yet

Some day this will be a parent's decision, per child, per server — that's
the design. Right now there's nothing to turn on: the per-player flag that
would let sats show up is fixed **off** in the game's code, and there's no
menu, command, or setting anywhere that changes it. If you see older
material (or an early build) mention a "guardian Bitcoin flag" being live,
that's describing the plan, not something switched on today.

## Proof of Play — real maths, not a payout

Every pickaxe strike runs a real **HMAC-SHA256 hash** — the same kind of
maths Bitcoin mining uses. Press **F3** to watch it happen. This is there to
**show you how proof-of-work actually works**, by doing it, and it also
drives the chunk obfuscation that keeps buried ore hidden from x-ray-style
cheats. The hash itself has never paid anything out, on any world, ever — it
just runs, visibly, for the education and the anti-cheat.

For the design spec, see `docs/spec/06-bitcoin-integration.md` (a design
document — nothing in it is live).

## A word on safety

Axe'n'Stax is **not a money transmitter**. The platform is designed never to
hold your funds — the intention, once real Lightning settlement exists, is
that sats flow straight to your own wallet, never through a central
AxeNStax account. Right now that settlement step isn't built at all, so
nothing moves as real money in any direction: you can't lose money playing,
and today's game has no sats to show, let alone spend.

See **[Trade Value](trade-value.md)** for how the in-game economy scoring
works without any Bitcoin involved.
