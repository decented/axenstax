# Trade Value & the In-Game Economy

Axe'n'Stax has a built-in economy ladder that turns farming, cooking and
crafting effort into a trade value — a score, expressed in "sats" the way a
board game might use "points." **Right now, on every world, that's all it
is: an internal score. No real money, no wallet, nothing spendable — see
[Bitcoin & Sats](bitcoin-and-sats.md) for the full picture.**

## The complexity ladder

Every food (and many materials) carry two numbers:

- **complexity_tier** — how many processing steps separate it from
  raw inputs. 0 = raw drop, 5 = masterclass meal.
- **trade_value** — the default sats-score price. It's a flat function of
  complexity tier — no per-item overrides ship today (a Cake just
  gets the plain tier-5 default like everything else at that tier).

The `/tradevalue` command in chat prints the trade value of the
item you're holding.

| Tier | What it means | Examples |
|---|---|---|
| 0 | Raw — drops or seeds | Wheat, RawBeef, Coal, Bone, Sapling, Berries |
| 1 | One workstation step | Bread, IronIngot, CookedBeef, BakedPotato, Flour, Bucket |
| 2 | Two-step processed | Cheese, Butter, Sugar, Dough |
| 3 | Three-step baked | SweetBread, Cookie, Pancakes, BeetrootSoup |
| 4 | Complex multi-ingredient | Stew, PumpkinPie, BerryPie |
| 5 | Masterclass | Cake, LoadedBakedPotato, Satori |

## Cooking premium

Cooked meat is **3×** the raw value (RawBeef is tier 0 at 1, CookedBeef is
tier 1 at 3). The hunter → cook → vendor loop:

1. **Hunter** kills a cow, sells RawBeef at 1 per piece.
2. **Cook** buys raw + fuels a furnace + sells CookedBeef at 3.
3. **Market-stall vendor** rents space + takes a flat **5%** slice on
   each Sell/Bulk trade.

Three actors making a living from the same source meat — all of it scored,
none of it real money.

## Whether the score shows as "sats" at all

A server operator can flip a `bitcoin_enabled` setting, and there's a
per-player flag that's meant to gate whether a signed-in kid sees sats
framing at all — that's the design for a future parent-controlled toggle.
Today that per-player flag has no switch anywhere: it's fixed off in the
game's code, for every player, on every server, regardless of the operator
setting. So right now trade value always shows as a plain in-game score,
never as sats framing, and Vendor Block's Barter mode (swap items, no
numbers) always works regardless.

## Tipping a Plaque

When a procgen village uses a community-authored house design, the
build's Architect's Plaque shows the chain of authors who designed
it (Original → Licence → Derivative). Right-click → **Tip** credits
score to the original architect's npub in the in-game ledger — an in-game
credit today, not a real transfer (see [Bitcoin & Sats](bitcoin-and-sats.md)).

Tip is a one-shot one-way credit; no escrow, no royalty curve.

## The Genesis Block

The very first **Satori gem** any player on a server drops is the
**Genesis Block** moment. Whoever gets that first Satori drop triggers
a special audio cue + toast — a one-time celebration, not a payout of any
kind. Only one Genesis Block per world, ever.

After it fires, the world's `genesis_block_found` flag flips to
`true` and the fanfare never fires again — Satori keeps dropping and
trading normally at its tier-5 value.

## More of the economy

Beyond Vendor Blocks and villager quests, a few other economy systems
exist, all scored the same way:

- **Bazaar** — a stateless server merchant that always buys your
  stack, no haggling needed. It pays trade_value × count, guaranteed
  — a reliable liquidity sink when there's no player buyer around.
- **Auction** — a placeable Auction Block that runs timed,
  reserve-price auctions. Bidding near the deadline extends the
  timer (anti-snipe), so you can't win by sniping in the last second.
- **Bounty / Mob Bounty Board** — a placeable board that posts daily
  mob-kill contracts for score.
- **Commission** — hire an NPC Builder to construct a Plan for you in
  exchange for a fee, instead of building it yourself.
- **Market Hub** — a Market Bell that rolls up every nearby Vendor
  Block into a single directory and powers a `/market` compass hint,
  so you can find every stall in the area at a glance.
