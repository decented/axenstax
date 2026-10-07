# C3 — the server owns a joiner's inventory (design, 2026-10-07)

**Status:** DESIGN DECIDED (session, real-multiplayer epic). Built in phases C3a → death → sidecar → C3b → C3c → C3d.
**Evidence:** `.claude/epic/handoff/c3-evidence-a.md` (containers, economy, armour, tool claims) and `c3-evidence-b.md` (local uses, death, layout, arrival, wire), both on main 6b17f617; `c2-evidence.md` §A/B/E/F.
**Specs that change as each phase lands:** Spec 04 §4.2d–§4.2g and §5.3.2; Spec 05 §4 (inventory), death and containers.

## 1. Where we are

A joiner's inventory today is the joiner's own. The server keeps a log-only **shadow** (C1) that follows the gains it sees and the costs it is told about:
- gains: pickups, break yields, grants
- costs: plain placements, D2b interactions, C2a eats, and C2b crafts and Q-drops

It never refuses anything. The evidence shows the gap is wider than the shadow-gap list in Spec 04 §4.2e:

- **Nothing persists.** Both sides start a joiner EMPTY on every join and rejoin. The client resets to `PlayerSlot::new` and never saves a joined session. The server builds a fresh `ServerPlayer` per attach. The dedicated server's `WorldSave.players` rows are write-only, and they leak the first joiner into the legacy player-0 fields. A joiner loses everything when they leave.
- **A joiner's containers are private, unsaved copies.** That covers chests, furnaces, the dispenser, composter, drying rack, hive, item frame and campfire cooking. No container contents cross the wire. Each joiner can loot its own copy of an untouched worldgen loot chest while the server's copy stays full. On a lent world the host's containers already ARE the truth; a joiner just can't see them.
- **Local uses are invisible:** 15 block-edit uses (bucket, seeds, bone meal, salt, papyrus, friction, eraser, rubber tap, hoe, flint, print hang, Plan Build and others), plus fishing (a client-seeded roll that mints items), bow and slingshot ammo, carts and face attachments. All of them classify `Unchecked`. A placement whose hand claims any tool is `Unchecked` too, so enforcing `check_placement` alone would leave a one-claim bypass.
- **The server has no armour, wears no tools, and believes** `InputPacket.armour_points` (bounded only by the 80 % reduction cap), `MinedBlock.tool` and the `EntityAttack` held claim. `EntityAttack` carries no hotbar slot.
- **Death is client-only:** a grave or scatter in the joiner's own world, never pushed. A grave the host makes reaches a joiner as a block id only, so opening it "recovers" nothing and desyncs.
- **The server→client wire is additive only** (`InventoryGrant`, `consume_held`). There is no slot-set, snapshot, container view or durability-set, and `WireItem` has no Plan.

## 2. The decision

**The server holds each joiner's inventory slot-for-slot as the truth. The client predicts it with the same rules, the way Minecraft's window clicks work.**

- **The window.** It is the 36 slots, 4 armour slots, the hotbar selection, the cursor, the craft grid, and the slots of one open container.
- **Shared rules.** Every inventory click, drag, sort, lock, trash, armour swap and result click is one pure state transition on a shared `Window` model.
  - The client applies it at once.
  - It also sends it as a window op.
  - The server applies the same transition to its copy, in arrival order.
- **Lockstep.** Because both sides run the same transitions in the same order, their layouts stay in lockstep by construction. Grants stay additive and land in the same slot on both sides (same `add_item` from the same layout).

**Why not the cheaper fork,** a layout-free "hold at least one anywhere" check (c3-evidence-b §3)? Two needs rule it out:
- A joiner's arrangement must survive leave and rejoin through the sidecar.
- Shared containers need server-ordered, slot-accurate moves between a chest and the player.

Only a server-owned layout gives both. Don't reopen this.

**Ordering.** Window ops are requests: they wait behind the same client's earlier edits (FU4a) and are processed in arrival order with its inputs. That is what keeps placements, auto-refill and ops in the same order on both sides.

## 3. Rules that hold across every phase

1. **Mirror first, enforce last.**
   - Phases C3a–C3c only mirror (log-only, tallied).
   - These four are **flip-time only** (C3d), because before then an honest joiner's locally filled bucket or chest-looted armour would not exist on the server:
     - armour points from the server's armour slots
     - attack damage from the server's held item
     - the mined tool from the server's slot
     - mismatch-driven resync with replay
2. **The sync carrier is sent only where the client can't be mid-op** until C3d: at join (from the sidecar) and at death.
   - A sync covers the whole window: 36 + 4 armour + hotbar slot + cursor + craft grid.
   - It carries no replay rule before C3d.
   - Before C3c a sync never destroys a client-held Plan; the client re-seats it in the first free slot (§5).
3. **Layout divergence sources are mirrored, not ignored:**
   - `auto_refill` (a per-client setting; the shadow assumes true today)
   - session locks, sort and trash
   - the hotbar slot of each edit's OWN input. Add it to FU3's `EditGroup` beside the hand. `check_placement` and server tool wear key on it, not on the latest input's slot.
4. **Graves stay ownerless,** Minecraft-like: anyone can take dropped items.
   - `GraveData` is a Vec element inside APPEND-ONLY `WorldSave.graves`. Adding a field to it would change every element's encoding.
   - If an owner is ever wanted, it goes in a side table appended to `WorldSave`.
5. **Regulatory.** `joiners/<npub>.dat` is local to the operator, and the npub is pseudonymous. It holds the inventory, armour, health, hunger, spawn point and night mark only. The host is the operator under red line 3. AxeNStax collects nothing.
6. **Economy blocks** (vendor, tip jar, auction, market hub, bazaar, bounty board, commission, repair bench) act on a joiner's private copy today.
   - From C3d a joined client refuses them with a toast.
   - Mirroring them belongs to a later economy lane, together with the five `LocalPlayer(pidx)` owner enums converging on `Npub`.

## 4. Phases

| Phase | What lands | Wire | Notes |
|---|---|---|---|
| **C3a: window mirror** | 1. Extract craft_ui.rs's click/drag/armour/trash/result-click transitions onto a shared `Window` model, unit-tested on plain values; egui drawing stays put. This is the main cost: judge-rung, high ambiguity. 2. C→S window op for the PLAYER window, including the craft result click. That click calls C2b's `judge_craft`, and C2b's `ItemAction::Craft{grid}` BRIDGE is retired. 3. The server holds 4 armour slots, the cursor and the craft grid, mirrored. 4. Sort, lock, trash and `auto_refill` are mirrored. 5. The hotbar slot goes on `EditGroup`. 6. Server tool wear on processed breaks and accepted swings, mirrored and log-only. 7. S→C `InventorySync` (whole window), sent at death only for now; join waits for the sidecar. 8. Log-only mismatch tally: the client sends a window digest and the server compares. | new op + sync packets; WireItem unchanged | C2b's grid/cursor owed-search BRIDGE goes. The client keeps predicting; nothing is refused. |
| **Death server-side** | Hook `mark_dead` (all three entry points: hazards, reported delta, reported death) for server-simulated players off keep-inventory. The server empties its window into a GRAVE plus `insert_grave` in the real world, or scatters real items if no safe cell. The client's death sweep is gated on `joined()`. The client receives the sync (empty). Grave retrieval by a joiner becomes a request (server `restore_to_inventory`, then sync). A reported death also sends `DiedOf`. | retrieval request | Must land before the sidecar. |
| **Sidecar** | `joiners/<npub>.dat` on the operator's machine, loaded at join accept (`verified_pubkey`) and sent as the join sync. Saved on leave and with the world. The dedicated server stops writing joiner rows into `WorldSave.players`, ending the p0 leak. Guests (no npub) stay ephemeral. The client's fresh-world kit/Bellows is suppressed for joiners; the server grants the testlab kit at first join. Restores health, hunger, spawn point and night mark. | join sync | Spec 04's "per-npub sidecar step". |
| **C3b: shared containers** | Open, view and ops on the server's real block entities: chest (and Sort, Dump matching, Restock, Take all), dispenser, furnace slots, composter, drying rack, campfire fuel/cook/pickup, item frame, hive. The server creates the entity on first use. A container window is part of the same window model. Joined clients stop opening private copies (closes the worldgen loot-chest dupe). Joined clients stop running the composter/keg/hopper/hive/autocollect sims. | container open/view packets | Absorbs the "joiner block-use" backlog for campfire fuel; the keg fuse goes with C3c's local uses. Steed pack and villager trade stay D2c. |
| **C3c: local uses** | The 15 block-edit uses become USE edits tagged with their own input's hand slot and use kind. The server applies the shared rule (`bucket::fill_result` and so on) to its slot. Plan Build reserves its materials as one op. Fishing rolls on the server. Bow/slingshot ammo, carts and face attachments go through requests. The keg fuse becomes a block-use request. | tag kinds | After C3c no `consume_one_material`/`add_item` site on a joined client is unmirrored. |
| **C3d: the flip** | Refuse placements and requests the server's window can't pay for, and resync on refusal (now with replay: the sync carries the last applied op and input seq, and the client replays later predicted ops). Derive armour points from the server's armour, ignoring the wire field for joiners. Attack damage and the mined tool come from the server's held slot. Economy blocks are refused for joiners with a toast. | sync gains replay seqs | **Acceptance (grep-able):** no `Unchecked` classification is reachable for a server-simulated player's edit; no inventory-mutating site on a joined client is unmirrored; the owner's playtest shows a near-zero mismatch tally first. |

## 5. Plans

`WireItem` has no Plan, and a Plan body can reach about 160 KB (the packet cap is 64 KiB).

- **Decision: by reference.**
  - In C3c the client reports a minted Plan (capture, latent print lift, kit) by content hash.
  - The server holds an opaque Plan marker in that slot, and the sidecar saves the marker.
  - The client keeps the body in a local store keyed by the hash.
- **A Plan can't leave its holder for another player:** no deposit into shared containers, no drop and no trade. Each of those is refused with a toast.
- **Plan transfer** (chunked bodies in both directions, so a Plan becomes a real shared item) is a later feature.
- **Until C3c** a sync re-seats any client-held Plan instead of destroying it.

## 6. Open (owner)
- None blocks C3a. A joiner's death drops and graves follow Minecraft-like ownerless rules. Say if graves should belong to their owner; that would be a side table.
