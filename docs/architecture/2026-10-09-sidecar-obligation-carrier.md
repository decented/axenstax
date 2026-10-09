# Sidecar obligation carrier (OC) — as built

Status: BUILT 2026-10-09, protocol v88 (provisional until merged), joiner save v2.
Lane: OC (C3d Decision 10's stop). Spec: Spec 04 §4.2i "Obligation carrier",
the `InventorySync` row of the packet table, and the v88 version-history entry.

## The defect it closes

`CorrectionDebt` (`container_window.rs`) lets a container correction's take and
give cancel in either order: a take that finds its item already gone leaves
units owed, and the next correction give of that item pays them first. There is
one copy on the server (`ServerPlayer.window_events.debt`) and one on the joined
client (`GameState.window_inbox.debt`), kept by the same rule.

On accepted main v87 the joiner save (v1) held only the effective window and
vitals, and the join's `WindowEvent::Replace` + `InventorySync` reset both
copies' debt. A later correction give that should have paid an old debt landed
as a free item. OC carries the debt across the reconnect.

## The contract as built

1. **One fold.** `window_events::effective_window_and_debt(sp) -> EffectiveFold
   { window, debt, saturated }` folds the waiting events into the window and
   the debt by the same rule (`apply_to`). With nothing waiting it returns a
   clone of the raw debt. `effective_window` delegates to it, so its callers
   don't change. `JoinerSave::capture` takes both window and debt from this one
   fold. It never pairs the effective window with the raw debt: a waiting
   correction give that pays the debt would then be counted in the slots while
   the debt also survived, a double charge at the next give.

2. **Replace carries the debt to both copies.**
   - `WindowEvent::Replace { window, debt }` sets each copy's debt to the carried
     one.
   - `InventorySyncPacket.owed: Vec<(WireStack, u32)>` (v88, after `slots`)
     carries the same debt.
   - The client's Sync apply (`inventory_sync::apply`, called from the `Sync`
     arm in `game_loop.rs`) installs exactly that debt.
   - The order of the owed list is kept exactly: it decides which entry a give
     pays first.
   - A join passes the restored save's debt; a fresh joiner passes its own
     effective debt, which is empty.
   - A death passes `CorrectionDebt::default()`, explicitly, at
     `hosted_server/death.rs`. This keeps the accepted Mirror behaviour (since
     v86) that a death clears debt, and a test pins it.
   - `send_inventory_sync` sends only what a carrier holds: `carried()` keeps at
     most 256 entries, and `wire_safe()` keeps only entries the client decodes
     back exactly. Both copies get the same carried debt. Anything left out
     marks the session `Overflow`.

3. **Versioned record, dispatch before decode.** `JoinerSave::decode` reads the
   leading version byte, then decodes that layout alone, with fixint encoding, a
   64 KiB limit and trailing bytes rejected. Byte 1 goes to the retained
   `JoinerSaveV1` (field for field the v87 layout, with its own version check),
   which is upgraded to empty debt and `Incomplete(LegacyV1)`. Byte 2 is v2. Any
   other byte is an error. The reader never falls back from one layout to the
   other. The writer always writes v2, and `JoinerSave::encode` runs `decode` on
   its own bytes first: bytes the reader would refuse are never written, and the
   older save stays.

4. **v2 layout.** The v1 fields in v1's order (`version`, `slots`, `armour`,
   `health`, `hunger`, `spawn`, `run_id`, `slept_night`), then
   `owed: Vec<OwedEntry { item: SavedSlot, units: u32 }>` and
   `obligation_history: ObligationHistory`.
   - `ObligationHistory` is `Complete | Incomplete(IncompleteCause)`.
   - `IncompleteCause` is `LegacyV1 | Overflow | Unreadable`, append-only.

5. **Obligation history.** `ServerPlayer.obligations: SessionObligations` is
   fresh per attach (`Complete`). It only ever becomes `Incomplete`, keeping
   the first cause. Nothing turns it back to `Complete`.
   - **`LegacyV1`:** the session's save was v1 (`restore_vitals` merges the
     save's history).
   - **`Unreadable`:** the npub is in `failed_joiner_reads` at load (amendment
     A1). Its saves stay refused, as before.
   - **`Overflow`:** the server copy's live debt passes 256 entries, or its
     units saturate. This is checked after each real apply: `apply_front`'s
     correction arm and `apply_own_now` (A2, A4). A capture also writes
     `Overflow` whenever its own fold passes 256 entries or saturates.

   Every capture writes the session's history, so a session that began
   `Incomplete` writes `Incomplete` at every capture.

   The history is a plain `&mut` field rather than an interior-mutable one,
   because the marking happens at the apply sites, which hold `&mut sp`; the
   save paths stay `&self`. If a capture's fold passes the cap without the live
   ledger ever doing so (an `apply_own_now` payment overtaking waiting
   corrections), that record says `Overflow` and a later, complete fold may
   write `Complete`. That later record is then the whole truth, so no debt is
   lost.

6. **Bounds.** A save is unreadable as a whole, and never truncated, if any of
   these holds:
   - it has more than 256 owed entries (`MAX_CARRIED_DEBT` =
     `protocol::MAX_SYNC_OWED`);
   - an entry has 0 units;
   - an entry's item is `Empty`, a Plan, or not exactly one unit;
   - two entries hold exactly equal items.

   An unreadable save takes the existing path: the file is preserved, saves are
   refused, and the session is `Unreadable`.

   The packet's bounded deserializer applies the same rules on the wire. It
   rejects a list over 256 entries, a 0-unit entry, a count other than 1, a Plan
   by kind or by full item, and two equal `WireStack`s.

   A capture whose fold passes 256 entries writes the first 256 in live order,
   marks `Overflow`, and logs the npub and the number left out. It never fails
   the save over this.

7. **Item encodings.** Units are always a separate `u32`, never a `u8` stack
   count.
   - **Save:** the item is a one-unit `SavedSlot`, the save's own encoding. It
     is exact for every non-Plan item, durability included, at any durability.
   - **Wire:** the item is a one-unit `WireStack` (`inventory::stack_to_wire`),
     the encoding every correction take already travels in. It is exact for
     every item a window holds: every registered block, every material, and
     every tool and armour kind at durability 1 to its maximum.

   A tool or armour piece at durability 0, or above its maximum, does not
   round-trip the wire. No window holds one, because wear removes a broken
   piece. The server leaves such an entry out (`wire_safe`) and marks the
   session `Overflow`.

   The client refuses the whole Sync as a protocol error (amendment A6) if
   it can't read the owed list whole. It applies neither the window nor the
   debt, and leaves the world with `HOST_BAD_WORLD_DATA`. This happens in two
   places:
   - a packet the bounded deserializer refuses ends the session in
     `RemoteClient::poll`, and is never silently dropped;
   - an entry that doesn't decode, or two entries that decode to one item,
     end it in the Sync apply (`inventory_sync::apply` returns an error to the
     `Sync` arm in `game_loop.rs`).

8. **The predicate is provided, not consumed.**
   - `JoinerSave::obligations_proven_zero()` is true when the history is
     `Complete` and nothing is owed.
   - `ServerPlayer::obligations_proven_zero()` is true when the session history
     is `Complete` and the effective debt is empty. It is false for an
     unreadable save's session, which is `Unreadable`.

   Nothing in OC refuses a join on either predicate. C3d's Strict admission
   and the handover's eligibility will consume them.

   `JoinerSave::from_parts(window, health, hunger, spawn, run_id, slept_night)`
   lays out a save with no debt and `Complete`, for a window that never had a
   correction debt: a host's trusted local slot (A7). `capture` is built on it,
   so no other code lays out the record by hand.

## Measured sizes at the cap

Measured by `joiner_save::oc_tests::oc_worst_case_save_and_sync_fit_their_caps`.
The worst case is every window slot holding a Plan (the largest slot encoding),
four armour pieces, and 256 owed tools and armour pieces (the largest non-Plan
item encodings) at `u32::MAX` units each.

| Item | Measured | Cap |
|---|---|---|
| Joiner save | 6,417 bytes | 65,536 bytes |
| `InventorySync` packet | 6,359 bytes | 65,537 bytes (tag included) |

The 256 cap stands.

## Notes for the consumers (C3d, handover)

- **"Proven zero" means no carried obligation, not that debt was never cleared.**
  A death writes off in-life debt, which is accepted Mirror behaviour. A save
  that died and respawned reads `Complete` with nothing owed.
- **A v1 save locks a joiner out of Strict for good.** Its session is
  `Incomplete(LegacyV1)` and every save it writes stays so. The only escape
  today is deleting the file, which yields a fresh `Complete` session with a
  fresh kit. This is an owner question for C3d.
- **A dead body can carry debt.** When a lending host releases a joiner while
  the server is not lent, `settle_leaving_life` is skipped, so the save can hold
  a dead body and live debt together. At rejoin that debt survives into the
  respawned life. The window and the debt stay consistent. This path predates
  OC.
- **Fishing cooldown is not carried.** C3d moves it to SIDECar v3.

## Deviations from the brief

- **Two item encodings (item 6).** The save uses `SavedSlot` and the wire uses
  `WireStack`. Each is an existing encoding and each is exact for what it
  carries. The wire is not exact at durability 0 or above maximum, which no
  window holds.
- **A fifth, canonical-form rule.** On top of the four bounds, an owed item must
  be exactly one unit, on disk and on the wire. This keeps wire equality the
  same as item equality, and keeps units out of the `u8` count.
- **The truth table's "unreadable" row is a session row.** An unreadable save
  has no `JoinerSave`, so the row is evaluated on `ServerPlayer`, through the
  `Unreadable` cause (A1).
