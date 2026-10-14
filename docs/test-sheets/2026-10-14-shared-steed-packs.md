# 14 October 2026 — shared donkey and mule packs (automated gates passed)

**Tested combined checkpoint:** `08e031b6231a9fbb9d6a7d25795a8f5470887000`,
tree `c663d5d37815787eecee0df3d9fcab9d7caf1930`. Engine v0.2.27, wire 91,
WorldSave 53 fields, save-layout revision 2 and canonical participant save v3.
The earlier integrated source checkpoint was `818e2691`.

**Automated acceptance:** `./check.sh` passed all 12 categories, including
6,525 native tests; all 112 GPU controls and all 214 real-ecryptfs handover
controls passed on that same checkpoint and test executable. Native game
binary SHA-256: `8cb452573345b8ee1293ec11e5f107b0d1bc0234b81a293abb203c9a669a25e2`.
Native test executable SHA-256:
`02bc75515cdd564e7e54d22adca3ee4f701e0b1d392be5a7ef59b204d79b8ea0`.
This documentation-only acceptance update follows the tested checkpoint;
it does not claim a new executable. Optional website smoke and 11 ignored
measurement/fixture-generation or live external relay/router/mailbox controls
were not run. The manual checks below remain pending. Mirror remains the
default and Strict remains partial/parked.

Final tested commit / native platform / host mode / tester:

Packs scoped source: `3924651eefb17e59b524c12d4bd81b38c432ed40`, normally
integrated into the combined source checkpoint. Its focused results and limits
below remain distinct from the final complete regression gate.

Use a disposable survival world, a signed host, two native joiners and one
Chest per equip attempt. Keep a donkey, mule and horse available. Record the
starting Chest and cargo counts so a failed interaction can be checked exactly.

## Shared interactions

- [ ] First ride of a wild donkey as a signed player keeps it. Ride it with a
      second player: the original keeper remains unchanged.
- [ ] Hold a Chest and equip the kept donkey, then repeat with a kept mule.
      Exactly one Chest is consumed for each pack. A horse refuses the pack.
- [ ] Sneak with an empty hand to open the 27-slot pack. Deposit and withdraw
      distinctive items; the host and both joiners see the same live stock.
- [ ] Two joiners try to take the last item. Only one receives it; the other
      sees the current stock and can continue playing.
- [ ] Change stock locally as the host while a joiner has the pack open. The
      joiner's next click uses the current state without restoring old stock.
- [ ] Remove an empty pack: receive exactly one Chest, including when the bag
      is full. Removal of a non-empty pack refuses and preserves all cargo.
- [ ] Race a deposit against empty removal. Whichever happens first determines
      the outcome; account for the item, cargo and returned Chest together.

## Live targets and persistence

- [ ] Walk with the steed while still in reach, then move out of reach. The
      open view follows the correct entity and closes/refuses when too far away.
- [ ] Leave a normal chest near the steed. Pack clicks never affect that chest.
- [ ] Reconnect as the keeper at another player slot, then restart the host.
      The keeper, pack, item positions and metadata remain intact.
- [ ] Try a guest on a guest-open host: riding a wild steed leaves it unkept;
      an already-kept pack remains usable under the normal access rules.
- [ ] Check an older local/split-screen steed save. Its legacy keeper is not
      automatically transferred to a joiner occupying the same numbered seat.

## Death and interrupted actions

- [ ] Destroy a packed steed in the disposable world. Open views close, cargo
      drops once, and a late click cannot restore the steed or duplicate cargo.
- [ ] Die while a pack interaction is in flight, then respawn. No late reply
      restores the old bag or changes the new life; the pack remains usable.
- [ ] Save and reopen an ordinary local pack containing a Plan. A locally
      available body retains its identity and development state; a missing body
      remains an unresolved marker rather than disappearing or changing identity.
      Joined pack deposit of a Plan remains refused.

## Visual check

- [ ] Pack title, slots, item counts, selected/focused control and refusal text
      remain readable at desktop and narrow native window sizes.
- [ ] Open/close and move items using mouse and keyboard/controller. The live
      pack never reverts another player's change when the local screen closes.

Notes / clipped text / difficult actions / unexpected counts:

This sheet does not authorise a push, a Strict default flip or a solo simulation
routing default flip. Automated conservation, ordering and durability tests are
reported separately with the final accepted build.


## Scoped automated evidence and limitations

The pack checkpoint passed native Clippy on unchanged production source. All eight
new cargo race, live reach/death/respawn, authenticated keeper reconnect/restart and
canonical private-body controls passed: seven at `f490a8e7`, then the keeper
control at `3924651e` after correcting its simulation-cleanup timing. The original
focused run was **50 passed / 1 failed / 5 ignored**; the single corrected keeper
retry passed. No all-green rerun is invented. Exact names, source commits and logs
are in `.claude/epic/logs/d2c2/checkpoint11-eight-success-provenance.tsv`.

All five new GPU controls executed successfully in earlier bounded checks: four at
`6c39bd41` and the lent-host view control at `98e9c13e` after correcting the guest's
interaction position. These results cover actual joined round trips in both modes,
ordered overflowing-prefix feedback, old-life late result refusal, local live
cargo changes and a host change pushed to an open joiner. They do not replace the
final combined source's full GPU suite. Earlier genuine causal assertion failures
and exact source restorations cover backed payment, prefix conservation, held-flight
ordering, bounded buffers and overflow feedback; the two fixture mistakes above
receive no feature-negative credit.

The final combined checkpoint passed the complete library, full GPU, real-disk
handover durability and `./check.sh` gates recorded above. Independent source
review found no outstanding HIGH or MEDIUM issue. Owner manual boxes above
remain unchecked; these automated results do not replace that session.

New pack payments are source-backed in both advertised modes; existing unrelated
Mirror bridges and global Strict recovery remain partial/parked, and Mirror stays
the default. Access requires any keeper plus live reach/entity/world policy; keeping
is not a new keeper-only cargo ACL. Signed first keeping uses verified npub. Guests
can ride wild steeds without durable keeping and use an already-kept pack. Legacy
local keeping is not silently transferred to an npub or a reused connection slot.

Joined Plan transfer remains refused. Local full bodies must be retained through
the checked private store before marker replacement/save; unknown bodies remain
honest unresolved markers with their development state. Bag/sidecar/world writes
inherit per-file atomicity without a cross-file crash transaction. A crash between
those files remains a limitation; these controls do not claim cross-file durability.
Dedicated creature life/AI and ordinary villager quests remain separate work.
