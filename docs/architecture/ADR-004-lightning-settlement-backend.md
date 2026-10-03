# ADR-004: Lightning Settlement Backend — phoenixd vs LNbits + Amboss

**Status**: Accepted — **fork 1 resolved NON-CUSTODIAL (owner decision, 2026-06-10).** Forks 2–3 follow from it. Backend pick revised to phoenixd-forward.
**Date**: 2026-05-24 (custody/settlement model resolved 2026-06-10)
**Relates to**: ADR-002 §"Bitcoin Model: Hybrid", CLAUDE.md §"Key Architecture Decisions" (Payments / Liquidity lines), `docs/research/2026-03-05-payment-architecture-chat.md`, `docs/spec/06-bitcoin-integration.md`

> **Scope note.** This ADR is about the *settlement backend* — the infrastructure that actually moves sats once a Bitcoin-enabled server owes a player. It does **not** affect alpha: alpha sats are notional/accounting-only and route through `economy::apply_sats_payout`, which is the single seam any backend plugs into (see the economy-layer audit, 2026-05-24). This decision becomes load-bearing only when real value flows on Bitcoin-enabled servers — post-alpha.

## ✓ Resolution (2026-06-10) — NON-CUSTODIAL, operator-funded spine

Fork 1 is decided. The owner's call (2026-06-10): **non-custody is a hard requirement.** The platform and the server-operator must never hold funds that legally belong to a player pending transmission to anyone else. This resolves the 2026-05-24 conflict in favour of a *new direction* — past even Spec 6's "operator holds the pool" model — and elevates ADR-002's noncustodial-wallet north-star from "someday" to the actual design. The custodial draft body further down (the 2026-05-24 "decided crux" + "Decision (proposed)") is **superseded**; it is corrected in place below and the original is preserved in git history.

### The decided model

1. **Players hold their own external wallets.** No app-controlled per-player custodial sub-wallets. The engine tracks an in-world **score** (sats earned through Proof-of-Play), never a stored balance of the player's money. Between worlds, a player's sats live only in their own wallet — the platform persists nothing withdrawable.

2. **Two independent one-way payments, never a held balance.**
   - **Entry is a nonrefundable, one-way payment.** The instant a player pays to enter, it is the operator's revenue — *not* a deposit. (Refundable entry = holding their money = custody. **This is the hinge.**)
   - **Payout is a separate payment *for work*, triggered by exit.** The in-world score crystallises into a real Lightning payment only when the player exits. It is a payout for work done, not a withdrawal of "their" money.

3. **Custody red lines (never do either):** (a) refundable entry; (b) a persistent, withdraw-on-demand balance that spans sessions/worlds. Either one turns the operator into a custodian/transmitter. The closed-world, nonrefundable-in / payout-on-exit design avoids both.

4. **Option B is the canonical, default spine** — *most legally defensible*. Entry buys access / covers costs; rewards are the operator **paying for Proof-of-Play work out of its own funds**, not a pass-through of pooled player stakes. There is no pot of player money being redistributed, so there is nothing to transmit.

5. **Option A (entry stakes ARE the redistributed prize pot) is permitted only as a genuine, non-custodial DLC peer tournament** — *never* as server-held escrow (escrow = custody). Caps, set by the payout structure:
   - **Continuous "keep what you mined" payout: ≤ 4 players** (the outcome space is near-continuous; pre-signing more is impractical).
   - **Winner-take-all / fixed bracket: ≤ 8 players** in one contract; brackets of 2-party matches scale beyond.
   - Above the cap, or for any large server, **decompose to per-player-vs-house 2-party contracts — which converges back to Option B.**

6. **DLC = the non-custodial settlement primitive.** Funds lock in a player-co-controlled multisig; the game server is the **oracle that attests the result (a signed Nostr/Signet event), never the custodian of the funds.** It can select the payout branch but cannot touch or redirect the money. Multiparty DLCs don't scale (combinatorial outcomes + all-online liveness) — which is *why* large servers fall back to Option B.

7. **Fee model — Lightning only touches the two doors.** Everything in-world is fee-free game state (1-sat granularity is fine — it's a score, not payments). Real Bitcoin moves only at **entry** and **exit**, ~0.4% per door on Phoenix/phoenixd, plus occasional receiving-side channel cost. A **minimum payout threshold** is forced by the receiving-side dust/liquidity floor (a few hundred sats for an existing wallet; higher, or operator-subsidised, for a player's first-ever receive). All fees disclosed up front. Rake destinations: network (Phoenix/ACINQ), server costs, promoter/creator, optional platform. This is also the cure for the "Wallet-of-Satoshi dust" antipattern — one meaningful payout per visit, never per-action dustings.

8. **Mitigations stack** (defence-in-depth, not one trick): nonrefundable + non-custodial + one-world-contained + low amounts + disclosed + skill-not-gambling framing (Spec 6 §10). **Counsel must bless the final structure before stakes grow.** Option A leans hardest on the skill-not-gambling defence — regulators read *substance over form*, so a pure pooled-redistribution still carries gambling flavour regardless of the "nonrefundable" label.

### Backend consequence (revises the Decision below)

Going non-custodial **removes the one reason this ADR originally picked Option C (phoenixd ⊂ LNbits)** — the need for many app-controlled per-player sub-wallets. With in-world balances now pure engine game-state, the backend mainly needs to *receive entries* and *pay out to external wallets with automated liquidity* — **phoenixd's sweet spot (Option B in the backend menu).** LNbits is demoted from "required custody layer" to *optional operator-side split/payout orchestration* (paying the server/promoter/platform rake). **phoenixd-forward; LNbits optional.**

---

## Background — the 2026-05-24 conflict this resolved (historical)

This ADR was blocked from 2026-05-24 until 2026-06-10 on the **custody / settlement model**, because the body below (the phoenixd-vs-LNbits decision *and* the wallet model) conflicted with the revenue-economics model specced in `docs/spec/06-bitcoin-integration.md`. The analysis that surfaced and framed that conflict is retained below for history; it is now answered by the Resolution above.

### The conflict (three axes)

| Axis | Spec 6 (canonical source-of-truth) | Owner's 2026-05-24 description + this ADR's draft body |
|---|---|---|
| **Custody** | Players hold **non-custodial EXTERNAL** wallets (§1.5, §7.7 auto-sweep). Operator holds only an in-world *ledger* + pool. | **AxeNStax-controlled custodial in-game wallets**, derived from identity keys. |
| **Pooling** | Operator runs an LNbits **pool**; the §5 split routes payments *into* 4 pool wallets via internal transfers. Platform never touches it. | **No central pool** — "the moment it goes to a pool, that's money transmission." Direct peer-to-peer payout. |
| **Who bears MT risk** | The **server operator**, in their own jurisdiction (§1.6, §10). Platform posture = non-handling. | **AxeNStax**, accepted at de-minimis with disclosure + low amounts. |

Per CLAUDE.md, **Spec 6 is the source of truth and must stay consistent** — so we cannot leave ADR-004 contradicting it. Either the owner's model is a *new direction* (→ Spec 6 §1/§5/§6/§7 need surgery) or Spec 6 already captures the intent and ADR-004's body needs correcting back toward it.

### Likely reconciliation (my read, not decided)

- At **alpha, AxeNStax is the only operator** (launch-scope decision), so AxeNStax-as-operator *is* the pool/ledger holder. The owner's "custodial, low amounts, disclosed" may simply be "AxeNStax-as-operator during alpha," while Spec 6's "operator in their own jurisdiction" is the **multi-operator future**.
- The word **"pool" may be double-counted**: Spec 6's **reward pool** is the *prize fund that pays mining rewards over time* (must accumulate — can't be pure pass-through). The owner's "no pool" was about *split routing* (entry fees shouldn't sit in a platform pot before division). **Both can be true at once.**
- "No pool" split routing is achievable via **Podcasting-2.0-style value-splits** (one payment fanned out as simultaneous direct keysend payments) instead of payment → pool → redistribute (Spec 6 §5.3 as-written).

### The deep-dive findings (what already exists — Spec 6)

- **Revenue split (§5).** Four destinations, three templates. Standard: **Reward Pool 50 / Creator (operator/YouTuber) 30 / Platform (AxeNStax) 15 / Reserve 5**. Creator Enterprise 40/45/10/5; Community 65/15/15/5. Floors enforced: pool ≥20, platform ≥5 (non-negotiable), reserve ≥2, creator may be 0.
- **Identity-tied recipients (§5.4).** Every non-pool recipient binds an LN Address to a Nostr pubkey via a `kind-11888` event — players verify "30% to creator" traces to a real identity (anti-scam).
- **Creator referral** (youtuber-flywheel doc): join-link tagged with creator pubkey → default 5% of referred players' mining → creator. Alt split there: Player 65 / Operator 20 / Creator-referral 5 / Platform 10.
- **Charity backstop (§7.6).** *This is the owner's "no rug" mechanism, already specced.* Mandatory charity beneficiary (identity-tied LN address) per Bitcoin server; on closure or 30-day abandonment, orphan pool + ghost balances route to charity (not operator/platform). Explicitly disincentivises ghost-balance gaming + profit-motivated closure. Shown on server-info screen; curated list at `axenstax.com/charities`.
- **Transparency (§7.5).** Treasury Transparency Panel shows cumulative split totals (advertised vs actual); identity-tied recipients; splits + charity on the server-info screen.
- **Auto-sweep (§7.7).** Pushes in-world balance > ~10K sats out to the player's external wallet (LNURL-withdraw).
- **MT posture (§1.5/§1.6/§10.5).** Platform non-handling; closed-world principle (§1.4) seals each world's pool; proof-of-work-not-gambling framing.
- **Server-tax helper** (sat-flow vision doc): `apply_server_tax(amount, kind)` single seam, default skim 0% on alpha. (Engine analogue today: `economy::apply_sats_payout`.)

### Owner's described model → mapped to what exists

- ✅ **Promoter / YouTuber cut** → Creator share (30%) + referral system. Exists.
- ✅ **AxeNStax org cut** → Platform (15%, ≥5% floor). Exists.
- ✅ **Charity-on-failure / no rug / disincentivise profit-closure** → §7.6. Exists almost verbatim.
- ⚠ **"Server farm / infra serving it" as a cut** → **GAP.** Not an explicit split line today; infra is paid from the operator's Creator share / hosting subscription / BitLaunch "from transaction flow." Owner wants a *transparent dedicated infra destination*.
- ⚠ **"All voluntary"** → conflicts with the mandatory floors (platform ≥5%, pool ≥20%). Transparency exists; "voluntary" doesn't, for those floors. Needs owner intent (operator-configurable %, or genuinely no platform floor?).
- ⚠ **"If the *project* fails"** → §7.6 covers *per-server* closure only. A **platform/org-level** charity backstop (AxeNStax-the-org winds down) is **not specced** — may be the real ask.

### Decisions needed to unblock — ANSWERED 2026-06-10

All three forks are resolved; see **✓ Resolution (2026-06-10)** at the top. In brief: fork 1 → **non-custodial (new direction)**; fork 2 → **operator-funded payouts (B) by default**, pool-and-redistribute only inside capped DLC tournaments; fork 3 → (a) infra/server cost is an explicit disclosed rake destination, (b) an org-level wind-down backstop remains a separate non-blocking item. The clarifying points are settled: it *was* a **change of direction**, and "AxeNStax controls the wallets" is **dropped** — non-custodial is permanent design, not alpha-only. Related memory updated.

---

## Context

The reward layer (Proof-of-Play, payout splits, Charter/server gating) is well-developed and backend-agnostic. The genuinely hard, under-specified part is **Lightning liquidity management**: having the *outbound* capacity to pay thousands of players, ensuring recipients have *inbound* capacity to receive, and absorbing the on-chain cost of opening/closing/rebalancing channels. This is distinct from "computing how many sats are owed" — it is the operational treadmill that sinks most Lightning projects.

### The incumbent plan (what we'd be revising)

From `docs/research/2026-03-05-payment-architecture-chat.md` and the CLAUDE.md tech-stack lines:

- **LNbits** as the integration hub — sub-accounts per user, keysend split routing, LNURL.
- **Amboss** (Magma) for inbound liquidity (~0.5%/tx + ~$5/mo modelled), with a mandated ~1% Lightning fee baked into splits.
- **Channel balancing handled operationally** — i.e. by hand / scripts.
- The **"mining pool"** model: sats accumulate in the server's pool (part of the split), and on player exit their portion routes to a wallet via keysend.

Two problems with the incumbent plan, both surfaced by the owner (2026-05-24):

1. **The operational burden is real and recurring.** "Channel balancing handled operationally" is doing a lot of work in that sentence. Running LND/CLN + buying Amboss leases + rebalancing is a standing ops commitment, not a one-time setup.
2. **It quietly creates custody.** LNbits sub-accounts are *custodial* wallets — the platform holds player balances. The "pool until exit" model means the platform transiently custodies funds. That is in direct tension with the project's hard constraint: **the platform must never be a money transmitter** (ADR-002, CLAUDE.md).

The suggestion on the table: **phoenixd** (ACINQ's headless Phoenix server daemon).

## Options

### Option A — LNbits + own node (LND/CLN) + Amboss *(incumbent)*

Run a full node, manage channels, buy inbound liquidity from Amboss, expose accounts via LNbits.

- **Liquidity**: manual. You decide topology, lease inbound, rebalance.
- **Multi-account**: native (LNbits' core strength — many sub-wallets, extensions).
- **Custody**: custodial sub-accounts → **money-transmitter exposure**.
- **Ops burden**: high and continuous.
- **Maturity**: very mature, MIT-licensed, huge ecosystem.

### Option B — phoenixd direct

Operator runs phoenixd; payouts go out via its HTTP API to player-owned external wallets (Lightning Address / LNURL / Bolt12).

- **Liquidity**: **automated.** phoenixd uses ACINQ as its LSP — inbound is just-in-time (channels open on the fly), channel management is automatic. This is exactly the "balance loading" pain, largely outsourced.
- **Multi-account**: **none natively.** phoenixd is one seed / one logical wallet.
- **Custody**: operator holds only the *operator's* float; players receive to their *own* wallets → **clean non-money-transmitter posture.**
- **Ops burden**: low — fund it on-chain, call `pay`/`receive`.
- **Maturity**: newer; single-vendor dependency on ACINQ as LSP; single-node throughput/availability questions at scale.

### Option C — phoenixd as the funding backend *under* LNbits *(was recommended pre-2026-06-10; superseded by the non-custodial decision → Option B shape)*

LNbits for the account/orchestration layer, phoenixd as its funding source — auto-liquidity from phoenixd, multi-account + split tooling from LNbits.

- **Liquidity**: automated (phoenixd/ACINQ).
- **Multi-account**: native (LNbits).
- **Custody**: **depends on how player wallets are modelled** — see below. If LNbits sub-accounts hold player balances, the custody/MT problem returns. If LNbits is used only for *operator-side* accounting + split orchestration and players receive to external wallets, the posture stays clean.
- **Ops burden**: low-to-moderate.
- **Maturity**: LNbits is adding/maturing a phoenixd funding source — **status to verify** (see Open Questions).

## The player-wallet model — SUPERSEDED 2026-06-10 (was "the decided crux")

> **Superseded by ✓ Resolution (2026-06-10) above.** This section had specified an **AxeNStax-custodial** wallet model — app-controlled identity-derived wallets, per-session **escrow**, and the thesis that "the money-transmitter escape is the settlement *topology*, not non-custody." That trade-off was **reversed** on 2026-06-10: **non-custody is now a hard requirement**, server-held escrow is forbidden (escrow *is* custody), and the spine is **operator-funded payment-for-work (Option B)** with **capped, non-custodial DLC peer tournaments** for the staked variant (Option A). What carried forward: no platform pool; wallets tied to Nostr/Signet identity; disclosure + low amounts as stacked mitigations; and the **non-custodial north-star — now the actual direction, not a someday.** The original custodial text is preserved in git history (pre-2026-06-10).

## Comparison

| Dimension | A: LNbits+LND+Amboss | B: phoenixd direct | C: phoenixd ⊂ LNbits |
|---|:--:|:--:|:--:|
| Liquidity management | Manual (high) | **Automated** | **Automated** |
| Inbound provisioning | Amboss leases | JIT (ACINQ LSP) | JIT (ACINQ LSP) |
| Multi-account / splits | **Native** | None | **Native** |
| Money-transmitter posture† | Risky (custodial) | **Clean** | Clean *if* external wallets |
| Ops burden | High | **Low** | Low–moderate |
| Vendor dependency | Amboss (swappable) | ACINQ LSP (harder to swap) | ACINQ LSP |
| Scaling model | Big node + many channels | Single node (throughput?) | Node + LNbits layer |
| Maturity / risk | **Very mature** | Newer | Integration newish |
| Recurring cost | ~0.5%/tx + flat | ACINQ liquidity/service fees | ACINQ fees |

† **Revised 2026-06-10:** under the now-decided **non-custodial** model (§"✓ Resolution"), the relevant column is **B (phoenixd direct, clean/external)** — players hold their own wallets, so "money-transmitter posture: clean" applies directly. The custodial-sub-wallet reading is superseded.

## Decision (accepted 2026-06-10)

The custody/settlement model is the driver; **✓ Resolution (2026-06-10)** above is the authoritative statement. In short:

1. **Non-custodial, player-held external wallets.** The engine holds an in-world *score*, never a custodial balance. **Nonrefundable** entry (operator revenue at the door); **payout-for-work triggered on exit.** No persistent withdrawable balance. (Refundability and cross-session withdrawable balances are the custody red lines.)
2. **Option B (operator-funded payment-for-work) is the default spine** — the most legally defensible shape. **Option A** (staked prize pot) is allowed **only** as a **non-custodial DLC peer tournament**, capped at **4** (continuous payout) / **8** (winner-take-all), *never* server-held escrow.
3. **phoenixd-forward backend.** With no per-player custodial sub-wallets required, phoenixd (auto-liquidity, pay-to-external-wallet) is the funding/payout node; **LNbits is optional** operator-side split orchestration for the rake — no longer a custody requirement. Liquidity fallback (LND/CLN + Amboss) unchanged.
4. **Lightning only at the two doors;** everything in-world is fee-free game state. Minimum payout threshold forced by the receiving-side dust floor; all fees disclosed up front.
5. **Keep `economy::apply_sats_payout` as the abstraction seam.** The backend lives behind it; this choice must not touch gameplay or gating code.
6. **Counsel before stakes grow.** The posture is nonrefundable + non-custodial + one-world + low-amounts + disclosed + skill-not-gambling (defence-in-depth); Option A leans hardest on the gambling defence.

## Rationale

- **The wallet model is now phoenixd-shaped.** With no app-controlled per-player custodial wallets, the backend just needs to receive entries and pay external wallets with automated liquidity — phoenixd's core competency. The per-player-sub-wallet requirement that made this ADR pick LNbits/Option C is gone.
- **Liquidity is still the bottleneck, and phoenixd still outsources it.** Trading manual channel ops for an ACINQ LSP dependency is a good trade for a small team whose moat is the *game*, not node operations.
- **The MT escape is non-custody itself, not a topology trick.** Reversed 2026-06-10: rather than accept custody and lean on no-pool topology, we removed custody — nonrefundable entry (purchase, not deposit) + payout-for-work + player-held wallets. The most legally defensible shape, and it matches ADR-002's non-custodial north-star.
- **The seam already exists.** Because alpha forced everything through `apply_sats_payout`, the backend is swappable; this decision is reversible at low cost if ACINQ terms or phoenixd maturity disappoint.

## Open questions / to verify before launch (engineering + legal)

1. **LNbits phoenixd funding source** — current maturity/stability, and whether it supports the split/keysend flows we need. (Decides C vs B.)
2. **phoenixd fee schedule + ACINQ LSP terms** — liquidity/service fees move fast and are past the Jan-2026 knowledge cutoff. Model against the mandated ~1% fee to confirm margins.
3. **Scaling shape** — shard architecture means many world instances. One phoenixd per shard fragments liquidity; a central payout service (one/few nodes) all shards call is likelier. Confirm phoenixd single-node throughput + HA story, or design the payout service as a queue in front of it.
4. **ACINQ dependency risk** — single LSP. What's the failover if ACINQ changes terms or has an outage? (Amboss is more swappable; weigh this.)
5. **Identity-derived wallet provisioning** — how an LNbits sub-wallet is deterministically bound to a player's Nostr/Signet identity (key derivation path, recovery, what AxeNStax holds vs derives). Ties into the Spec 1 Phase 4 signing bridge.
6. **Regulatory sign-off** — confirm with counsel that the **custodial-but-direct-settlement, disclosed, de-minimis** posture stays outside money-transmitter / e-money definitions in target jurisdictions, and identify the threshold (amount / user count) at which it no longer does. Cross-reference `project_bitcoin_parent_controlled` (parental control) and the UK OSA considerations. This is the load-bearing legal question — the no-pool topology is necessary but not obviously sufficient given custody.
7. **Disclosure surface** — where/how the app-controls-your-wallet trade-off is communicated to players (and guardians), since disclosure is one of the two conditions the model rests on.

## Consequences

- **`docs/spec/06-bitcoin-integration.md`** reconciled 2026-06-10: new **§1.5 "Custody & Settlement Posture"** records the non-custodial, nonrefundable-entry / payout-for-work model authoritatively; §5 (splits), §7.7 (auto-sweep) and §10 (regulatory) carry pointer notes subordinating them to it. A fuller mechanical rewrite of §5/§6/§7 (retiring the pool-and-redistribute math in favour of operator-funded payouts + capped-DLC tournaments) is a tracked follow-up. **CLAUDE.md** "Payments / Liquidity" lines still to update: phoenixd-forward, LNbits optional, no-pool, non-custodial (the existing "Noncustodial Lightning wallet" line already matches). The "mining pool" framing is retired in favour of operator-funded payouts; Amboss/manual-rebalancing demoted to a liquidity-only fallback.
- **`docs/research/2026-03-05-payment-architecture-chat.md`** is left as historical context (it captured the original thinking); a pointer to this ADR added.
- No engine code changes implied now — `apply_sats_payout` stays the seam. A future foundation spec covers the actual phoenixd integration (HTTP client, payout queue, LN-Address resolution) when a Bitcoin-enabled server is first stood up.
- The `held_sats`/escrow BRIDGE comments across the economy blocks (Vendor/Tip Jar/Auction) point here: their notional withdrawals become real phoenixd payouts under this backend.

## What this does NOT change

- **Alpha is unaffected** — still notional sats, no real settlement.
- **Hybrid Bitcoin model** (ADR-002) stands — the game works fully with Bitcoin off; this only concerns servers that turn it on.
- **Proof-of-Play** as the reward source primitive is independent of settlement backend.
- **The not-a-money-transmitter constraint** still governs — and as of 2026-06-10 it is honoured via **non-custody itself**: nonrefundable entry (purchase, not deposit) + payout-for-work + player-held wallets, stacked with disclosure, low amounts and one-world containment. The earlier "topology + accepted custody" framing is superseded.
