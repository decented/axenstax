# C3d source checkpoint: authoritative admission and exact replay

Source base: accepted smallfix `1dc5725e`, protocol 87; rebased 2026-10-10 onto local main `bdd4fe87` (OC obligation carrier, protocol 88 delivered). This is the first source
and model checkpoint for an implementation in progress. The new modules are
not connected to gameplay dispatch yet. The initial compile and 21 model tests
pass. Six deliberately broken invariants produced eight assertion failures;
two later journal regressions also failed before their fixes. These are model
results, not actual Strict dispatch or applied world-intake acceptance. Strict
is not shipped or enabled by this checkpoint.
The approved delivery proposes protocol 89 (provisional; OC took 88) and operator-local
`--strict-inventory`, default false, fixed and advertised at join. Trusted local
host slots keep their existing policy. No discovery, service, identity collection,
monetary feature or public capability claim changes.

## Interfaces and integration order

`action_identity` owns the server-issued 128-bit namespace, checked sequence and
window epoch, exact terminal frontier and recovery cut. `action_queue` retains
one ordered stream and hashes bounded payloads for queued duplicate comparison.
A terminal duplicate has no mutation. Inventory execution completes only after
full staged commit and immediate numbered events/receipt/views are registered.
Movement acknowledgement never advances this frontier.

`action_journal` allocates before prediction and retains sent intents, pure
semantic recipes and touched/dependency cells, including unsent overlays. No
private Plan body is owned by an entry. Retirement requires included events and
an applied world boundary. Cuts cannot regress after earlier history is freed.
The included event base also cannot regress after a receipt or recovery; later
applied post-fence events are a distinct prefix and do not redefine that base.
The eventual receipt carrier must name authoritative included E explicitly,
not reuse the original action's client-reported events_applied as authority.
Recovery cuts include the connection namespace: coincident id/epoch/life/sequence
numbers from another session cannot acknowledge or activate them.

Append reserves the aggregate retained suffix context before sequence allocation
and prediction. The conservative sum counts duplicates too and stops at8192
cells, so several separately valid actions cannot make complete Describe
impossible. Receipt, recovery and death release the corresponding reservation.
Server-derived dependency headroom and the combined client/server footprint
still require an actual union/count/512 KiB check before bundle capture; the
model reservation does not authorise truncation or arbitrary context generation.

`staged_window` runs the actual `window::apply` on raw marker-only scratch
holdings. Full private bodies are converted to exact marker/developed stubs
before cloning. This avoids `Inventory::clone` copying resident private bodies.
Explicit branch sources cover slot/grid/cursor/armour, craft cells, drag inputs,
trash, unlocked sort inputs and initial Close/Autofill sources. Close/Autofill
conservatively include initial returned sources and bag sources to cover
intermediate return/merge/reuse. Simple clicked-stack requirements are currently
conservative; they will be narrowed to the actual branch quantity where needed
by honest crossing tests. This checkpoint does not claim complete container,
wear-normalisation, pending-debt or product-fit integration.

`SourceBacking` is transient evidence for one action: initialise each place from
raw R, observe EVERY prefix of the existing pending event/debt fold, reduce its
eligibility and never increase it for a later Grant. The real owed-search rule
is reused, not replaced by total holdings. A removed source cannot be backed by
an identical stack elsewhere or by a future same-slot Grant. The action itself
runs on R, never on effective E. Pending Replace, ambiguous owed wear or debt
attribution returns uncertainty before K; no external effect, cut K−1. Definitive
invalidity terminally refuses K, cut K. Post-commit mismatch cuts through K.

`action_capacity` reserves the complete immediate event/result/view footprint
before external commit: a two-event shot at 1023/1024 pending events creates no
projectile. It also exposes 16 MiB per-body and 64 MiB resident private-body
limits. Integration must count unique bodies AND every owned copy/capacity, with
at most one additional body decode in progress. Arc pointer deduplication alone
cannot bound many distinct bodies. Marker/store keys live in the journal;
actual durable body retention is required before history release.

`rebase_assembly` bounds counted owned pages. Begin/pages/End parsing does not
activate anything. Only an actual applied world-intake fence can make the
complete assembly ready. In integration, the fence enters `ClientOutbox`'s SAME
FIFO after preceding synchronous world captures; it clears coalescing indexes.
`RemoteClient` inserts a non-overtakable world delta at that stream position.
`chunk_intake` carries a blocked chunk AND all later steps/fence across polls.
Generate/restore required columns before applying authoritative cell tombstones,
then atomically install canonical world/window/context and rebuild the suffix.
No saved prediction block may overwrite a newer canonical value.

### Carrier integration checkpoint (source, not acceptance)

The evolving protocol 89 (provisional) carriers use Action=68, ActionReceipt=69, Recovery=70 and AppliedActionAck=71, appended after OC v88, which added no tag (it extended tag 67).
JoinAccept now appends fixed inventory mode, a fresh random128-bit action
namespace and life. Randomness failure refuses admission; an extra JoinAccept
cannot replace an existing journal or hot-switch mode. Mirror remains the
configured default; the Strict operator flag is not exposed at this stage.
The joined client has explicit reserve-before-predict and retained-send APIs;
actual gameplay mutation ingress and typed authoritative admission are still
pending, so these APIs alone do not prove coverage.

ClientOutbox stores a complete frozen recovery as one owned bounded queue
entry. Its Begin/pages/End/fence can span tick budgets while later world steps
wait. Recovery and ordinary action receipts clear both live and rebuilt
coalescing indexes. Overflow during a partial bundle removes its whole remaining
entry and reports the failed cut; the authority's restart/settlement integration
is still pending. A truncated capture cannot emit its later End/fence.
Overflow also removes ordinary receipts and reports the lost boundary before
dropping authoritative blocks: a receipt can never retire history ahead of a
resync snapshot queued later. The authority must consume both failure reports.
Normal late terminal duplicates are ignored only for the current namespace,
epoch/life, already retired sequence and an included E within the retired base.
They cannot change that base or release the surviving suffix; foreign or
conflicting receipt frontiers fail closed.

Describe uses Begin, pages of at most2048 cells and End, because8192 cells do
not fit one ordinary64 KiB wire packet. The whole client suffix remains bounded
by8192 cells and512 KiB including actual intermediate allocations. Complete
union with server-derived structural dependencies must still be bounded before
mutation/capture; cap-edge tests belong to the forthcoming typed handlers.

RemoteClient parses bounded recovery parts but enqueues the fence at the real
world-stream offset. ChunkIntake carries fence/receipts and every later step
behind a forced-column budget stall. GameState marks a fence only from actual
ordered world intake. Ordinary receipt retirement waits for that stream position
and authoritative included E; movement ACK and socket parsing retire nothing.
Canonical bundle installation, durable body release, ordinary-carrier freezing,
replay and Ack-before-resubmission remain OPEN. Actual outbox, intake and channel
dispatch regression tests were first compiled and executed on the OC base
(2026-10-10, `57818111`): 110/0 targets. Real `HostedServer` tests now cover
Strict admission, the action FIFO's exact duplicate result and the lost-receipt
teardown (`hosted_server/c3d_tests.rs`); twelve negative controls on the
admission source each failed an actual assertion and were restored byte-exact.
This is source/test evidence, not Strict acceptance: the typed breadth,
canonical install and replay remain OPEN and the operator flag is unexposed.

The wire integration will add one typed action envelope containing ActionHeader
and a bounded existing edit/op/request or compound/toggle/hold intent. It splits
consecutive edit batches at any intervening op/request. Legacy request/op counters
may remain diagnostic only through an explicit immutable bounded action mapping;
no request outcome/refusal may mutate Strict holdings by an unscoped legacy key.
Strict ignores old raw mutation carriers. Mirror also retains identity/journal
where needed for truthful diagnostics, without silently changing its bridge rules.

Recovery carriers name full namespace/id/epoch/life/cut: Prepare, bounded Describe,
Begin, counted pages, End, actual FIFO Fence and Ack. A new epoch executes nothing
before Ack, which precedes resubmissions on the ordered stream. Replay invokes
shared pure rules only, retaining original sequence and semantic expected item;
invalidated intent sends CancelledNoop at that same sequence. Accepted chest
moves/projectiles/mints at or before the cut are never resent. Included outcomes
are UI metadata; capture and restore accepted active cast id/target/remaining
bite/cooldown and hold id/target/slot/progress/freshness/reset or tombstones so
UI rebuild does not restart accepted rolls.

Death explicitly cancels old-life history through the highest allocated action,
including unsent predictions, via bounded session-scoped describe/cancellation.
The server cannot infer this number from movement ACKs or its last queued action.
The journal retains its next sequence; life2 seq11 is expected after cancelling
old-life1–10, even if the server executed only1–4. Preserve bodies durably, drain
accepted grants and run the existing one-grave hook. Generic recovery cannot ACK
past the newer death boundary. Fresh connection starts a new namespace and never
resends prior external effects.

## Bounds

| Retained object | Count and bytes |
| --- | --- |
| Client journal / server unified queued actions | 2048 actions AND 2 MiB each, including actual deque capacity, entry storage, encoded intents, recipes and context allocations |
| Individual encoded intent / ordinary packet | 32 KiB / existing 64 KiB wire cap; existing tighter packet/tag limits remain |
| Pending event staging | 1024 entries AND 2 MiB |
| Result page | 128 entries AND 32 KiB |
| Recovery describe | 8192 cells AND 512 KiB, including dependencies and unsent overlays |
| Complete owned bundle / individual page | 2 MiB including assembly storage / 32 KiB; at most64 pages |
| Private body / resident bodies and copies | 16 MiB / 64 MiB, one extra decode at a time |
| Recovery timeout / restarts at one cut | 200 server ticks / at most2, then truthful settlement and clear disconnect |

No bound evicts an unacknowledged action or manufactures an acceptance. At a cap,
stop new inventory prediction/admission and recover; movement and normal world
simulation remain live. Checked integer exhaustion closes/rejoins, never wraps.

## Actual mutation ingress audit at source base

All paths below are under `game/engine/src`. The interception must precede the
first prediction/cost, not merely packet send. These references identify current
seams, not claims that allocation has already been added.

| Client ingress | Existing authority seam | Required staged source/effect |
| --- | --- | --- |
| Mining/instant break and placement in `game_loop`; `window_ops::PendingEdits` | `hosted_server:4757 process_or_queue_edits`, `4832 process_one_edit`, `5354 classify_joiner_edit` | Historical held slot/tool, shared break replacement and exact placement cost/refill; loaded/reach/access; typed exhaustive classification before world mutation |
| Tagged use arms before `game_loop:4233 push_use_edit` / `4256 push_use_edit_costing` (currently post-mutation) | `process_one_edit`, `use_edits::judge/settle` | Table below; capture ActionSeq BEFORE cost/world prediction; Legal precedes generic Noop |
| Door placement and toggle arms | New compound/toggle dispatcher, using existing shared door metadata rules | One bottom intent, one door cost, two budget credits; complete companion permissions; derive upper, suppress old top/meta packets in Strict |
| `craft_ui:292 apply_click`, game-loop inventory dispatch around21321 | `window_ops:679 serve_op`, `window::apply` | Marker-only raw stage, branch sources, exact config/station ordering; no total-count or effective whole-click substitute |
| Container `craft_ui:227`, `game_loop:18823 apply_container` | `window_ops:821 serve_container`, `container_window::apply_container` | Current REAL bounded container and raw personal sources; commit both once, no client-view stock/deposit belief/durability swap |
| `game_loop:22255 send_entity_attack`, `22305 send_interaction` | Attack/interact handlers, `hosted_server:3354 run_joiner_interaction` | Actual historical hand capability, slot/wear/consume/product/fit; interaction before consume may not escape failed admission |
| `22343 send_eat_request` / `22376 send_drop_request` | Item dispatcher, owed Take | Named source, complete actual pay before heal/real drop; no raw/effective shortfall belief |
| `22522 send_plan_minted`; actual art/capture generation callers | Plan mint dispatcher / `2426 take_capture_paper` | Historical Blueprint Paper or CURRENT complete bounded connected paper; marker fit/payment before deletion/mint; art insertion-before-spend raw layout preserved |
| `22553 send_sleep_request` / `22730 send_open_grave` | Existing bed/grave handlers | Context/permissions and genuine window grants; existing death/grave origins remain once-only |
| `22583 send_block_use`, `22596 send_light` | `2746 serve_block_use` | Exact shared cost/tool, genuine products/fit, new typed server-observed friction state; lit Stick fuel remains ordinary |
| `22667 send_shot`, `22693 send_place_cart` | `2475 serve_shoot`, `2565 serve_place_cart` | Actual held weapon/cart and shared ammo search, wear, whole immediate events reserved before projectile/cart |
| `22758 send_fishing` | `2641 serve_cast`, `2688 serve_reel` | Historical rod capability, server seeded state/cooldown, once-only catch/wear; active cast idempotent |
| `22707 send_attach`, `22742 send_detach` | `2299 serve_attach`, `2374 serve_detach` | Existing ownership/solid/face/reach rules, complete paper cost or genuine recovered product; no believed Attach |
| `22623 request_creative_give`, Explorer callers17433/19908 | CreativeGive dispatcher | Authoritative Creative permission and bounded real fit; developed policy distinct from capture |
| `23116 send_device_interact` | Existing shared server device rule | Typed target and action identity, canonical server transition; no arbitrary meta authority |
| GrantUnfit callbacks; `23023 flush_window_ops`; `remote_client:1349 send_input` /1493 requests /1565 ops | Window event return routing and intake | Every inventory portion identity-scoped; authentic origin/backing once-only; late raw old-epoch upstream inventory inert, movement/chunk feedback retained |

## All standalone edit-use cost/capability rules

Shared `UseKind::ALL` has12 variants; DoorUpper is replaced in Strict by the
one paid compound primary, leaving11 standalone rules. Every row also requires
current life/loaded coordinates, primary reach and existing permissions;
structurally derived dependencies receive their own permission preflight.

| Use | Historical held source / spend / wear | Shared world predicate and product |
| --- | --- | --- |
| BucketFill | Named held empty Bucket, consume1 | Source water/lava; shared fill outcome, AIR; filled Bucket1, real fit |
| BucketEmpty | Named matching filled Bucket, consume1 | Shared empty rule at destination; authoritative fluid, empty Bucket1 |
| Sow | Named matching seed, consume1 | AIR above tilled soil; exact shared crop |
| PlantPapyrus | Named Papyrus Reed, consume1 | AIR over valid dirt/grass/sand by water; shared stage0 |
| GrowGrass | Named Bone Meal, consume1 | Grass/support plus AIR above; shared tall grass |
| GrowCrop | Named Bone Meal or Fertiliser, consume1 | Actual authoritative crop and permitted shared next stage; preserve newer crop on stale refusal |
| Salt | Named Salt, consume1 | Shared acceptable surface; salt path |
| Erase | Actual named Eraser capability, wear1 | Existing Blueprint Paper; AIR, Papyrus Sheet1; AIR Impossible |
| TapRubber | Actual named backed held Bucket, retained (no consume/wear) | Live tappable log; tapped log, Rubber1; no generic no-cost exemption |
| Till | Actual named Hoe capability, wear1 | Shared dirt/grass top rule; tilled soil |
| HangPrint | Exact named developed Plan marker, consume1 | Valid wall/support and shared destination/orientation; print, no private body verification |

CaptureArt permits only latent `developed=false` markers. CaptureCommit's
marker developed state equals authoritative Creative mode, matching existing
body generation; otherwise latent. Never independently normalise its flag from
its private body/marker. The server validates paid source, footprint and fit,
not private geometry/author/licence/provenance. Missing paper creates no marker.
Current connected paper is consumed; no equality claim to dialog-time private
geometry. Builtins/CreativeGive keep their separate permitted developed policy.

## Debt proof and next coherent integration milestones

Strict begins at fresh attach from the truthful SIDECar window with zero
CorrectionDebt/forced phantom debt. It may never call Mirror's believed deposit,
R−P phantom repair, durability swap, partial paid shortfall or full-grant bridge.
Use current real container and source-backed raw transaction; contested take0
is terminal NormalisedAcceptedNoop through K followed by correction, not a
retry. Numbered strict grants/takes/wear are actual admitted effects, not debt.
Existing owed event application is reused; if its trace cannot assign a source,
recover before K. Assert zero bridge debt at every strict commit, recovery,
death/settlement/save boundary.

Decision 10 stop RESOLVED by the obligation carrier (OC, local main `bdd4fe87`,
protocol 88): SIDECar v2 carries the effective correction debt and a sticky
`obligation_history`; the join's Replace/InventorySync give it to both copies.
Strict admission (`strict_admission::check`, called in the real join path right
after `GameServer::load_joiner` and before `JoinAccept`) requires the record to
prove no carried obligation: no record is proven; a v2 record must be
`Complete` with no owed entry (`JoinerSave::obligations_proven_zero`); an
unreadable record (session `Incomplete(Unreadable)`), a v1 record
(`Incomplete(LegacyV1)`), an overflowed one (`Incomplete(Overflow)`) or any owed
entry is refused with a typed `StrictJoinRefusal` whose sentence is the
`JoinReject` reason. The handshake is not done, so `release_slot` neither
settles nor saves: the record's bytes are untouched. No settlement from held
stock, no write-off, no inference from slots. Mirror admission is unchanged.
Strict never creates debt: real `HostedServer` tests assert the live fold stays
empty and the capture proves zero (`hosted_server/c3d_tests.rs`). The old
source-only v1 counterexample test is removed; OC's real tests cover the case.
Decision 23 as amended: the verified-npub fishing remaining cooldown (clamped
100) goes in SIDECar v3 (v2 is the obligation carrier); v1 and v2 migrate to 0;
guests session-only. Active line on death/disconnect/world-leave imposes 100
ticks before SIDECar capture.

Next: typed protocol/advertisement and complete identity ingress; staged pending
projection/wear/product/container admission; server capture/outbox and client
canonical overlay/applied fence/replay; exhaustive gates including compound
permissions, server hold/fishing lifecycle; real HostedServer/GPU split-poll
controls and full stable-tree acceptance. No intermediate merge/default flip.
Automated acceptance cannot replace the owner's actual near-zero honest-play
session. Prepare a test sheet separating lockstep, container convergence, stale
world refusal, skipped baseline/Creative, frontier/replay failure counters.

## Wired through real dispatch (2026-10-10, second run, v89 provisional)

Strict stays internally selected (no operator flag), Mirror the default bridge.

- **Click.** `serve_strict_window` stages the server's EXISTING transition, shared
  with Mirror's `window_ops::serve_op` (`server_click_ctx`: table reach slack and
  grace; `server_click`: no Result the client refused, a refused Autofill reruns
  without the grace; `close_container_view` on the ops that close a view). Order:
  marker raw stage with prefix source backing and pending fit → digest → result
  row reserved → raw commit → post-commit fold guard. A digest the raw transition
  doesn't reproduce is `Refused` at its seq with zero effect (fail-closed
  teardown until recovery is wired). A window the marker projection can't carry
  exactly (a broken tool, an unregistered block, a clamped durability) is refused
  with a typed fate before commit; nothing is dropped or clamped; the projection
  is fallible (`MarkerWindow::try_capture_parts`, `staged_window::exact_marker`),
  so a pending `Grant` of such a stack no longer panics. A Plan becomes its
  marker placeholder (marker and developed state kept).
- **Shoot.** `serve_strict_shot`: the outcome is serialised first; the Take and
  Wear are reserved together (`prepare_strict_events`), then the result row; a
  result-row failure gives the event reservation back exactly
  (`PreparedEvents::abandon`, deque capacity restored). Only then the one
  projectile, the two numbered events, the row and the outcome.
- **Strict never creates debt.** Staging refuses a fold that owes before any
  effect (logged, distinct reason); every Strict commit (click, shot) is followed
  by `strict_fold_holds` over `ServerPlayer::obligations_proven_zero`: a breach is
  logged and the session torn down with a reason, never a panic or a write-off;
  a Strict departure with a non-empty fold is logged and saved as it stands.
- **Client Esc close.** `EscAction::CloseCraft` goes through
  `GameState::close_crafting` (reserve before predict, no legacy `WindowOp`).

Still open: shared containers in Strict (typed `OpenContainer` with a reserved
`ContainerView`, container click staged on the REAL container, contested take →
`NormalisedAcceptedNoop`, `event_heap_bytes` sizing a `ContainerView` instead of
refusing it, `container_push` views reserved); typed window configuration
(`OpenPlayer`/`OpenTable`/`SetAutoRefill`), which an honest Strict client still
sends as legacy ops; every other typed class; recovery/replay.


## Shared containers, configuration and review lows (run 3, 2026-10-10)

Module `hosted_server/strict_container.rs` (server) and the reserving entry
points in `window_actions.rs` / `craft_ui.rs` (client).

- **Configuration.** `OpenPlayer`, `OpenTable` (`window_ops::open_table_grace`,
  shared with `serve_op`) and `SetAutoRefill` are typed actions staged on the
  raw marker window. A Strict client sends `SetAutoRefill` before any window
  action whose digest covers it, and at session start (the flush), never as a
  legacy op (the Mirror op log is bypassed; a stray logged op is not sent).
- **Open.** The verdict is `HostedServer::container_open_verdict` (shared with
  Mirror). The opened view and the result row are reserved before the
  container opens. A refused open is Accepted with no effect and answered
  `OpenRefusal` (the client predicted none).
- **Container click.** `staged_window::stage_container` runs the shared rule
  on marker scratch twice: on `ContainerViews::seen` at the action's boundary
  (the client's predicted view; digest check, `Refused` on mismatch) and on
  the real container. Equal effect on every moved slot and the personal
  window → sources admitted across the pending prefix (`admit_staged`, the
  same check a window click uses) → result row → the real container and the
  raw window commit once. Otherwise `NormalisedAcceptedNoop`, zero effect.
  **Departure from the brief's wording:** a PARTIAL contest (fewer units than
  predicted) is treated as a contest too (whole-or-nothing), because without
  replay the client could not reconcile a partly-applied prediction.
- **Pushes** to Strict joiners reserve their view (`push_strict_container`).
  `event_heap_bytes` counts a waiting view's real bytes (slot storage plus any
  Plan body) instead of refusing every reservation behind it.
- **One teardown.** `HostedServer::strict_bridge_teardown` (BRIDGE: recover
  instead of teardown) is the only Strict close; the player reads
  `strict_admission::StrictTeardown::reason`.
- **Run-2 review LOW-1.** `window_events::strict_obligations` is the one
  predicate for staging and the post-commit guard: no correction debt, a
  `Complete` history, every waiting Take payable.
- **Run-2 review LOW-2.** A refused Autofill stages only the subset its client
  did (`stage_strict_refused_autofill`).
- **Review L2** (movement-packet edits fail closed), **L3** (encoded-size store
  limit; durability-before-cache kept for Mirror too, recorded in Specs 04/05),
  **L4** (forced-close retry paced on a reservation error). L4's identity
  cache for Plan markers is NOT built: `Item::Plan` holds `PlanData` inline, so
  an address-keyed cache is unsound under allocator reuse without a drop or
  mutation hook; needs a design call.

## Run 4 (2026-10-10): leftovers

Coordinator decisions on run 3's departures, as built:

- **Partial contest is whole-or-nothing** — accepted (Spec 04 records it).
- **Refused open** = an Accepted row with no effect plus `OpenRefusal` — accepted.
- **Short Take in `apply_front`.** A numbered Take a Strict copy applies short
  is an invariant breach. `window_events::apply_front` records its number
  (`WindowEvents::take_strict_short_take`; Mirror stays log-only) and
  `HostedServer::strict_events_hold` tears the session down through the one
  BRIDGE with the obligation sentence. Checked where a Strict action applies
  its boundary (before anything executes on top of it) and after every tick's
  overdue valve, which covers every other application point by the end of
  the tick.
- **Stray legacy op on a Strict client.** No whole-window resync request
  exists, so `remote_client::flush_strict_window_ops` makes the client leave:
  a graceful `Disconnect`, the session `Failed` with
  `STRICT_STRAY_CHANGE_REASON`, and the world exit discards the joined copy.
  Never a silent discard; nothing is sent as a `WindowOp`.
- **L4 Plan marker cache — deferred** to a final performance pass (with the
  2–3× fold repetition). Bounded cost meanwhile: each Strict click re-hashes
  every Plan body in the window (at most 50 places × that body's bytes, each
  body ≤ 16 MiB encoded), under the per-tick window-op head budget
  (`MAX_WINDOW_OPS_PER_TICK`). No address-keyed cache is built.
- **Run-2 review LOW-3/LOW-4.** The L5 test's doc now claims only what it
  proves; `c3d_actual_hosted_lost_receipt_teardown_settles_an_unfit_grant_once`
  proves settlement (one ground spill of a full bag's unfit Grant, a
  proven-zero save). Red controls were run for the shot post-commit check,
  `abandon` (the result-cap rig shrinks the deque first so the reservation
  must grow it), the L5 settlement, the Esc close and the table edge.
