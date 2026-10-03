# Claim — merch fulfilment intake

A tiny code-gated form that turns "I took your money / you won a prize" into a
shipped Printful order **without you ever typing an address or estimating
postage.** No payment runs through here — you take cash or sats in person; the
**code** is the proof of payment.

## The model

```
At the stand:  take cash/sats  ──►  hand over a single-use CODE (card / QR)
Later (or there): buyer scans QR ──► claim.axenstax.com/?code=XXXX
                  ► enters code, picks size, types THEIR OWN address
                  ► we POST it to Printful  ►  Printful prints & ships
```

- **No payment here** — by design. The code is what authorises one order.
- **You don't touch the address** — the buyer self-enters it; it goes straight to
  Printful and isn't stored here.
- **You don't estimate postage** — Printful bills *your* account for product +
  shipping. **Price your in-person sale to cover international postage** (Printful
  shipping to the EU/UK is typically several £ per item).
- **Abuse-proof** — the form is useless without a valid unused code, so a random
  visitor can't run up your Printful bill.
- **Safe by default** — orders are created as **drafts**; Printful neither charges
  nor ships until you confirm in its dashboard. Set `PRINTFUL_AUTO_CONFIRM=true`
  once you trust it for fully hands-off fulfilment.

This is a deliberate alternative to a full storefront (Shopify/Ecwid). It only
works because *you* handle payment; it does the fulfilment half via Printful's
v1 API (`POST /orders` with `sync_variant_id`).

## Setup

1. **Get a Printful private token** — Printful Dashboard → Settings → Developers →
   add a private token (with order scopes). Your 3 tees must already exist as
   **synced products** in that store (they do).
2. **Configure** `.env` (from `tools/sites/deploy/env.claim.template`):
   - `PRINTFUL_TOKEN=...` (required)
   - `PRINTFUL_STORE_ID=...` (only if the token is account-level)
   - `PRINTFUL_AUTO_CONFIRM=false` (keep drafts until you've done a test order)
3. **Run it**: `./start.sh` (or the systemd unit on the box). With no token it
   just shows "not open yet" — safe.
4. **Generate codes**:
   ```bash
   .venv/bin/python gen_codes.py 50 --csv > prague-codes.csv   # 50 codes + URLs
   .venv/bin/python gen_codes.py --status                       # totals
   ```
   Codes live in `data/codes.json` (per-host; gitignored; excluded from deploy rsync).

## Event kit

- **Buyers/winners** → a card or QR to `claim.axenstax.com/?code=XXXX` (the CSV
  gives you `code,claim_url` — drop the URLs into a QR generator and print cards).
- Each code is single-use; a failed order automatically frees its code to retry.
- **Test first**: with `AUTO_CONFIRM=false`, place one real claim, then check the
  draft order looks right in the Printful dashboard before the event.

## Files

- `app.py` — the form + POST handler (FastAPI, port 8100).
- `printful.py` — v1 API client (list variants, create/confirm order).
- `codes.py` — single-use code store (reserve/release/set_order, race-safe).
- `gen_codes.py` — generate / inspect codes (run on host).
- `templates/`, `static/` — the mobile-first claim page.

## Not included (on purpose)

Payment, refunds, tax/VAT collection, an order dashboard (the **Printful
dashboard is your order record**).

The planned upgrade path is **not** a hosted storefront (no Shopify) — it's a
**custom front end that accepts Bitcoin, fiat, and Monero**, fulfilling through
this same Printful API. This claim service is the in-person **v0** of that: the
order-creation half (`printful.create_order`) is already here and reused; the
online build just swaps "single-use code (proof of in-person payment)" for "paid
invoice webhook (proof of online payment)" as the authorisation to place an order.
BTC + Monero point at a self-hosted **BTCPay Server** (non-custodial, supports
Lightning + on-chain + XMR); fiat/cards are the one piece that pulls in an
external processor.
