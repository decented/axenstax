<!-- SOURCE: vendor.rs, tip_jar.rs, plot.rs, economy.rs, bazaar.rs, market_hub.rs, auction.rs, cart.rs, rail.rs, server_economy.rs, reserve.rs, proof_of_play.rs | Verified against code 2026-06-22. COMPLIANCE-CRITICAL — re-read 00-assistant-persona-and-rules.md §2.2 before using. -->

# 18 — Economy, Trading & Vendors

**The economy is a *sovereignty* feature, not an earning feature.** It's how you run your own shop, claim and protect your land, set up a market, and move goods around *your* world. Read this page together with `00-assistant-persona-and-rules.md` §2.2 — the money rules there are absolute and override anything that could be read as "earn money" here.

> **The one rule for this whole page:** there is **no real money** in the game today. Some of these features keep an in-game **sats** tally (a score-like number), but it is **notional / accounting-only** — nothing can be earned, withdrawn, or spent as real currency, real-money settlement is **deferred**, and any future version is **parent-controlled and opt-in**. **Never tell a player they can earn or make real money.** Frame all of this as *building and running your world*.

---

## 1. How to think about (and talk about) the economy
- These are **builder/operator tools**: a shop you own, land you claim, a market you run, a railway you build. That's the pitch — *your world, your rules* — never "come earn".
- The in-game **sats** number that some features use is a **game token**, not real Bitcoin and not real money. In code, every sats movement goes through one gate (`economy::apply_sats_payout`) that on alpha is **accounting-only** — it records a number; **no Lightning, no wallet, no withdrawal.** Real settlement is explicitly deferred.
- If a player asks "can I get real money out of this?": the honest answer is **no** — that's not what this is, it's switched off, and any future money feature is a grown-up's decision. (See `99-accuracy-and-deferred.md`.)
- **Barter needs no money at all** — it's pure item-for-item trade, and it's the friendliest way to frame trading to a kid.

## 2. What's actually in the game (verified)

| Feature | What it does | Status |
|---|---|---|
| **Vendor Block** | A shop block you place and own. **Sell** mode = others pay (notional) sats for an item you stock. **Barter** mode = others trade an item for your item (no money). | ✅ Sell + Barter live |
| **Tip Jar** | A block tied to you; others right-click to send you (notional) sats. | ✅ live |
| **Plot Marker** | Claims a fixed square of land (≈32×32) that **only you can build/break in** — anti-grief protection for your builds. Creative bypasses. | ✅ live |
| **Server Bazaar** | A "sell-floor" merchant: sell any held stack for its trade-value (notional sats) — guaranteed liquidity for any item. | ✅ live (sell-only) |
| **Market Bell / Market Hub** | Designates a market area; the `/market` compass points to it and lists nearby vendors — discovery + navigation. | ✅ live |
| **Auction Block** | A timed lot: set a reserve, others bid, highest bid wins at the deadline (with anti-snipe extension). Escrow is notional. | ✅ live |
| **Rail freight (carts, rails, depots)** | Lay track, load a cart at a depot, dispatch it along the rail, ride along. The cart is the engine's first vehicle. | ✅ live |

## 3. Vendors in detail (most common question)
- **Place & own:** craft and place a Vendor Block; you're the owner. The owner dialog sets the **mode**, the item slot, and the price.
- **Sell mode:** you stock an item; a buyer right-clicks and pays the set (notional) sats. (No real money changes hands.)
- **Barter mode:** you ask for one item in exchange for another — a pure swap, no sats. **Recommend this when helping kids** — it's trading without money.
- **Anti-grief:** breaking someone's vendor is protected.
- **Buy mode is DISABLED pre-alpha** — a vendor cannot buy items *from* a player for sats. Don't tell a player they can sell goods to a vendor for money.

## 4. Land, markets & logistics (sovereignty framing)
- **Plots** are the cleanest sovereignty story: *claim a patch of the world and nobody can mess with your build.* Lead with that.
- **Market Hubs** are about *running a marketplace* — cluster vendors, help visitors find them with the `/market` compass.
- **Rail freight** is *building infrastructure* — tracks, depots, carts, riding the rails. Great for the "build something bigger" itch.
- **Auctions** are *price discovery between players* — a fun timed mini-event, framed as trading, not gambling (it's an open ascending bid, not a chance draw).

## 5. Proof-of-Play & "rewards" — read carefully
- The **Proof-of-Play** hashing and the "Deepslate Reserve / richness" reward-pool model exist in code, but the **real-sats reward path is gated off and deferred** (see `11-mining-tools-blocks.md` and `99-accuracy-and-deferred.md`). The richness shown in alpha is a **synthetic placeholder**, not a live payout.
- **Never connect mining to earning.** Mining is for materials and building; the hashing is an educational/anti-cheat detail, not a way to make money.

## Deferred / not yet
- **Real-money settlement** (Lightning, wallets, withdrawals) — deferred entirely; the sats tally is notional/accounting-only. Parent-controlled and opt-in if it ever ships.
- **Vendor Buy mode** — disabled pre-alpha.
- **Bazaar buy-side** (buying *from* the Bazaar) — deferred to v2 (sell-only today).
- **Market-hub stall rentals**, **vendor multi-slot / reputation gating / stale-shop expiry** — deferred post-playtest.
- **Cross-server / hosted public marketplaces** — alpha is single-player/LAN.

---

### Assistant guidance for this page
- Lead every economy answer with **building/ownership/trading**, never earning.
- If "sats" come up, say plainly it's an **in-game token, not real money**, and real money is off + parent-controlled.
- Prefer **Barter** and **Plots** when guiding kids — they're sovereignty/trade stories with no money at all.
- If a player wants to "get rich", gently redirect to fun goals — a thriving shop, a protected base, a railway, a full trophy room — not money.
