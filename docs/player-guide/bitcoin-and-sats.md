# Bitcoin & Sats

You'll see the word "sats" mentioned around the game — in quest text, Vendor
Blocks, Plaques, the Furnace. This page is the honest, short answer to what
that means today.

## What a sat is

A **sat** (satoshi) is the smallest unit of **Bitcoin**. That's it — a unit
name, like "penny" or "cent."

## Switched off everywhere

The game itself has no Bitcoin in it. On **every** world, sandbox or server, **no sats show up, no real money changes hands, and there's no wallet.** Nothing in the game converts to anything spendable.

If a server's operator ever runs a Bitcoin layer, it is **their own, optional layer on their own server**, and it is **off by default**. The game itself does not turn anything you do into sats.

You don't need Bitcoin to enjoy the game. Items, reputation,
quests, villages, Knights — all of it works fully with sats off. Barter mode
covers trading without ever mentioning a number.

## Nothing to turn on

There's no menu, command or setting anywhere that switches sats on. The per-player flag that would let sats show up is fixed **off** in the game's code. If you see older material (or an early build) mention a "guardian Bitcoin flag" being live, that's describing an old plan, not something in the game today.

## Proof of Play — real maths, not a payout

**Proof of Play** is a real **HMAC-SHA256 hash** — the same kind of maths Bitcoin mining uses. It runs only in one place: when you break **natural deepslate**, deep underground, with a **Diamond pickaxe or better**. The hash decides whether that block hides a **Satori**, the rarest gem in the game.

Plain mining runs no hash. Grass, dirt and ordinary stone do not, and nothing on screen shows a hash.

It is there to **show you how proof-of-work actually works**. On a server, the secret behind the hash stays on the server, so nobody can peek at where a Satori hides. A Satori is a gem you keep and trade by Barter. It has no sats value, and it has never paid anything on any world. (Hiding buried ore from x-ray-style cheats is built but not switched on yet.)

For the design spec, see `docs/spec/06-bitcoin-integration.md` (a design
document — nothing in it is live).

## A word on safety

Axe'n'Stax is **not a money transmitter**. The game never holds your funds, and nothing in it moves as real money in any direction. You can't lose money playing, and today's game has no sats to show, let alone spend.

See **[Trade Value](trade-value.md)** for how the in-game economy scoring
works without any Bitcoin involved.
