# Full-fidelity item wire encoding (death-drops phase 3)

**Status: ✅ DELIVERED 2026-09-06** (spec'd 2026-07-12; built on
`solo-queue-2026-09-06`). Protocol **v60 → v61**, 17 new tests, `check.sh`
ALL GREEN.

**Deviations from the spec as written:**

- **Version numbers.** The spec said v58 → v59; weather sync took 59 and
  world chat took 60 in the interim, so this landed as **v60 → v61**. All
  three pin sites updated (`protocol.rs` const + its test,
  `test_integration/handshake.rs`).
- **`apply_inventory_grant` takes the packet, not loose fields.** Adding
  `full_item` as an eighth positional argument tripped clippy's
  `too_many_arguments` (threshold 7) under `-D warnings`, so the signature is
  now `(inv, ecs, player_pos, &InventoryGrantPacket, registry)`.
- **Server eligibility is `!matches!(item, Item::Plan(_))`**, expressed
  directly on the `Item` rather than by round-tripping through `item_to_ref`
  — the old filter's shape. Same rule, one less indirection.
- **Render fidelity was already tier-only on BOTH sides.** The spec expected
  the remote path to gain a pickaxe-shaped look; in fact
  `build_item_entity_vertices` (local) resolves a tool to
  `tool_texture(material)` too, so tools looked the same either way. The real
  render win is **armour**, which encoded `Empty` and so rendered *nothing*
  before. The shared resolution was extracted as `entity_model::item_textures`
  and is now called by both paths, so any future per-tool icon lands on both
  at once.
- **`WireItem` derives `Copy`/`Eq`** (all fields are integers) — it is passed
  by value throughout rather than by reference.
- **Live 2-client WebSocket probe not run** (spec phase 5, "optional"). The
  in-process `HostedServer` + `ChannelClientTransport` tests cover the same
  encode/decode path end to end; the genuine two-machine walk-over pickup
  remains an owner-side verification.

Original spec below, unchanged.

## TL;DR

Server-side tool and armour drops are invisible-in-practice to remote
players: the `(item_kind: u8, item_id: u16)` wire pair collapses a tool to
its bare material tier and an armour piece / plan to `Empty`, so the server
refuses to grant them (it would otherwise mint a wrong-kind, full-durability
item) and they sit on the server floor until lifetime expiry. This spec adds
a `WireItem` payload carrying **tool type + material + durability** (and the
armour equivalent) on `EntitySpawn` and `InventoryGrantPacket`, so a dead
steed's half-worn iron pickaxe reaches a remote player's inventory as
exactly that. **Plans stay floor-bound** (heavy `PlanData`; excluded by
design — the append-only enum leaves room to add them later). Protocol
v58 → v59.

This is the last solo-buildable item on the wave-hardening backlog
(`project_wave_hardening_backlog` memory). It completes death-drops phases
1/2a/2b (`docs/spec/05-gameplay-systems.md` §9.5 implementation note) and
the 2026-07-12 late-joiner backfill.

## Context pointers (verified in code 2026-07-12)

| What | Where |
|---|---|
| Lossy encoding: tools → tier-only, Plan/Armour → `Empty` | `inventory.rs:19` `item_to_ref` |
| Decode refuses `TOOL` refs + unknown ids | `inventory.rs:51` `item_from_ref` |
| Wire pair + `ItemRef` (NOT serde — always the `(u8, u16)` pair) | `protocol.rs:301` `item_kind`, `:313` `ItemRef` |
| Server pickup eligibility = `Block \| Material` only | `server.rs:770-778` closure into `entity.rs:995` `tick_item_pickups` |
| Grant packets filter the same way | `hosted_server.rs` `build_grant_packets` |
| Item spawns encoded on broadcast (TWO sites) | `hosted_server.rs` `diff_entities` item pass **and** `backfill_entity_events` item pass |
| Client ghost table skips `Empty` refs | `remote_entities.rs` `RemoteItems::apply` |
| Grant → inventory decode | `remote_entities.rs` `apply_inventory_grant` |
| Remote drop-cube render resolves textures from `ItemRef` | `entity_model.rs:1588` `build_remote_item_vertices` (local path `build_item_entity_vertices` already renders `Item::Tool` properly — reuse) |
| Types to carry | `crafting.rs:108` `Tool { tool_type, material, durability }`, `:137` `Tool::max_durability`; `armour.rs:58` `ArmourItem { slot, material, durability }`, `:140` `armour::max_durability` |
| Version pin sites (THREE) | `protocol.rs` const + its test, `test_integration/handshake.rs` (`assert_eq!(PROTOCOL_VERSION, 58)` today) |

## Design

New serde enum in `protocol.rs` (bincode-positional — **append-only
forever**, same rule as `EntityKind`):

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WireItem {
    /// No extra fidelity — fall back to the (item_kind, item_id) pair.
    None,
    Tool { tool_type: u8, material: u8, durability: u16 },
    Armour { slot: u8, material: u8, durability: u16 },
    // Plan { .. } is a FUTURE append — deliberately not carried now.
}
```

- `EntitySpawn` gains trailing `full_item: WireItem` (`None` for mobs/carts
  and block/material items). `InventoryGrantPacket` gains the same.
- Enum⇄u8 mapping by **explicit match both directions** (pattern:
  `item_to_ref`'s tier match) — unknown discriminant decodes to `None`,
  never a panic or an `as`-cast.
- New helpers in `inventory.rs`: `item_to_wire_full(&Item) -> WireItem` and
  `item_from_wire_full(&WireItem) -> Option<Item>`. Decode validation:
  durability **clamped to `max_durability`** for the decoded kind and
  **0-durability refused** (a tampered or newer-version peer must never
  mint an over-max or broken-but-granted item). Grants are server→client
  only, so this is defence-in-depth, not a trust boundary.
- Decode precedence everywhere: `full_item != None` wins; else the legacy
  pair via `item_from_ref`.
- Widen the server pickup eligibility (`server.rs` closure) and
  `build_grant_packets` filter from `Block | Material` to *everything
  except `Item::Plan`*. Plans keep today's floor-bound behaviour.
- Client: `RemoteItem` stores the `WireItem` alongside the pair;
  `build_remote_item_vertices` maps `WireItem::Tool/Armour` back to an
  `Item` and reuses the local `build_item_entity_vertices` geometry so a
  dropped pickaxe *looks like* a pickaxe (today it renders off the bare
  tier, wrong or generic).

## Phases (TDD — watch each fail first)

1. **Protocol**: `WireItem` + the two trailing fields + v59 bump. Tests:
   serde round-trip incl. unknown-discriminant tolerance; update the THREE
   pin sites. ⚠ Version-history doc gotchas: continuation lines must be
   4-space-indented and must never start with `+` (clippy
   `doc_lazy_continuation` reads it as markdown).
2. **Helpers**: `item_to_wire_full` / `item_from_wire_full` + clamp/refuse
   validation tests (over-max durability, zero durability, unknown bytes).
3. **Server**: populate `full_item` in `diff_entities` **and**
   `backfill_entity_events` (don't miss the second site — late joiners must
   get fidelity too); widen eligibility + `build_grant_packets`. Tests:
   TestHost tool-drop granted server-side with durability preserved; grant
   packet carries the payload; extend `test_integration/late_join.rs` to
   assert a backfilled tool spawn carries `full_item`.
4. **Client**: `RemoteItems::apply` accepts tool/armour spawns;
   `apply_inventory_grant` decodes `full_item` first; render mapping.
   Unit tests in `remote_entities.rs` mirroring the existing suite.
5. **Verify**: `check.sh` green; optional live 2-client WebSocket probe —
   recipe in `project_death_drops_phase2b_shipped` memory (temp `#[ignore]`
   test driving `RemoteClient::connect_websocket` against a real
   `--server` process; kill a steed carrying a pack near probe B).

## Acceptance criteria

- A tool dropped in the server sim (e.g. dead steed cargo, iron pickaxe at
  durability 37) renders as a pickaxe for a remote client and lands in
  their inventory as **Iron Pickaxe, durability 37** on walk-over.
- Armour pieces likewise, with slot + material + durability preserved.
- Plan drops behave exactly as today (floor-bound, not rendered) — no
  regression, no `Empty` ghost cubes.
- Tampered payloads (durability > max, durability 0, unknown discriminant)
  are refused; nothing is granted and nothing panics.
- All three PROTOCOL_VERSION pins read 59; `check.sh` ALL GREEN.

## Memory-rule check

- No player-facing money/earning words are introduced (word-guard tests
  unaffected).
- `WorldSave` untouched — this is wire-only; the append-only save invariant
  doesn't come into play.
- Wire enums (`WireItem`, `EntityKind`, `item_kind`) stay append-only.
- UK English in any new display strings (none expected).
