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
  doesn't reproduce is `Refused` at its seq with zero effect, then corrected
  by one live whole-window `Replace` (C3d run 12b, S1; "Refusal" below). A window the marker projection can't carry
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

## Run 4 (2026-10-10): run-3 review folds

- **M1 (liveness).** `window_events::strict_obligations` folds a waiting
  `Replace` (join or death sync) from the window it carries (its carried debt
  must be empty) instead of failing; source lineage across it still can't be
  established (`admit_staged` keeps `Replacement`). Configuration ops skip the
  contents digest while a replacement waits (`WindowEvents::replace_waiting`).
  `SetAutoRefill` bypasses the dead / death-sync refusal. The client's flush
  sends the session-start auto-refill only once event 1 (the join sync, when
  there is one) is applied; the entry points carry it otherwise.
- **L1 copy.** Every teardown sentence states what happened and ends
  "Anything this world saved for you is kept." (true for a guest and for an
  unreadable record too); no "update your game", no inventory promise.
- **L2 Plan store.** Encoded-size limit exactly main's (no 16-byte box in the
  reservation); Mirror's `remember` passes no residency (no 64 MiB cap);
  Windows uses a write-through rename and no directory sync. The
  `cfg(windows)` path is NOT compiled on this Linux host.
- **L3.** `HostedServer::select_inventory_mode` (the field is private)
  refuses Strict on a `--no-lend` host (`StrictSelectError::NoLendHost`).
- **L4 contest interleavings: NOT done this run** (same-tick, push in flight,
  container broken mid-contest) — next run.

## Run 5 (2026-10-10): superseded actions, contest interleavings, review folds

- **Stale-base and dead-body actions are superseded, not breaches**
  (coordinator decision; run-4 review M1 and LOW-1). In
  `HostedServer::process_typed_actions`, a non-configuration action from a
  dead body, or whose base predates the latest whole-window `Replace`
  (`WindowEvents::predates_replace`: `events_applied < last_replace`, the
  join or death sync), is completed with the terminal
  `ActionFate::Superseded` (wire tag 4) and zero external effect, never torn
  down. It replaces run 4's `SetAutoRefill`-only exemption; configuration
  (`SetAutoRefill`, `OpenPlayer`, `OpenTable`) stays served while dead or
  across a waiting `Replace`. A stale base below a `Replace` the overdue
  valve already applied is superseded too; any other unavailable base is
  still `FellBehind`. Residual (declared in run 5, corrected by the run-5
  review and closed in run 6): a client that applied the death sync and then
  predicts while still dead was superseded with no `Replace` left to
  overwrite its prediction. The death-driven container Close named here is
  harmless (container clicks never touch cursor, grid or station); the real
  cases were Esc on the death screen with a crafting screen open and an
  early Respawn with a table screen open (the force-close). See run 6.
- **Contest interleavings (run-3 review L4).** Same-tick withdrawals, a
  deposit contest with the winner's push in flight, and a chest broken
  between one joiner's commit and the other's click, each with conservation
  by identity over effective windows, the chest, graves and ground items.
- **LOW-2.** The server's model of a Strict mirror (opened view, pushes,
  `seen`) holds a Plan as `PlanData::placeholder()`
  (`container_window::strip_plan_body`, `ContainerData::without_plan_bodies`);
  the real container keeps the body. A view that still can't be reserved is
  `OpenRefusal::TooLarge` (wire tag 4), never a teardown.
- **LOW-4.** `strict_unsupported_here()` (owning server with local players)
  is checked at selection and at every Strict admission
  (`StrictJoinRefusal::NoLendHost`).
- **Windows (coordinator decision; LOW-3).** The `cfg(windows)` Plan-store
  write is `save::write_atomic` (file fsync + rename, no directory sync);
  run 4's `MoveFileExW` FFI is gone. The branch is uncompiled on this host;
  the directory-sync failure test and its seam are `cfg(unix)`.
- **Selector.** `select_inventory_mode`, `inventory_mode()` and
  `StrictSelectError` carry
  `cfg_attr(not(test), expect(dead_code, reason = "Strict operator flag — exposed at C3d acceptance"))`.

## Run 6 — typed block edits (2026-10-10)

- **Dispatch.** `ActionIntent::Edit` with no use tag, on a Strict slot, goes
  to `hosted_server/strict_edit.rs`. It counts against the tick's edit budget
  (`MAX_BLOCK_CHANGES_PER_TICK`), not the item counter.
- **Classification from raw R, never the claim.** The item in the action's
  historical hotbar slot (an invalid slot refuses; no latest-slot fallback)
  decides: a fill of AIR/water is a `Place` only when that item places exactly
  the block, otherwise `Refused` with zero effect; a mined break of a block
  that yields is a `Break` whose drop tier is the tool in that slot (none = a
  bare hand). `MinedBlock.tool` and the input's held pair are never read for
  the yield. Untagged emptying or in-place edits are `Refused`. Creative,
  same-block, door, non-yielding manual and use-tagged edits are paused as
  untyped (their typed classes come later).
- **Departure from the run-4 plan (recorded for the coordinator).** The plan
  said "numbered Take / WearWeapon". The client PREDICTS a placement and a
  break's wear at its boundary, through `take_placeable_from_hotbar` (which
  auto-refills the hotbar slot from the bag on the last unit) and
  `use_tool_at`. A numbered `Take` applies through `take_owed_window`, which
  never refills, and lands after the pending prefix the client predicted
  before. So the edit's window half is staged as a window click is
  (Decision 6, `stage_strict_click`): `window_events::stage_strict_edit` runs
  the shared transition on raw R, every changed place is a source at its
  whole initial value, every waiting event must fold onto the candidate as
  onto raw R (`admit_staged`), and the candidate commits as raw R. Only the
  break's yield (not predicted) is numbered: `Grant` events staged after the
  pending prefix on the candidate (`stage_strict_effects_on`), reserved with
  their packets and the result row before any effect.
- **Commit order.** Validation (`validate_block_edit`), judgement, staging,
  grant packets, event and result reservations; then the shared world half
  (`apply_edit_world` / `finish_edit_world`, split out of `process_one_edit`
  so Mirror runs the same code), raw R, the numbered grants, the terminal row
  and the grant packets; then the debt-zero guard.
- **Refusal (C3d run 12b, S1).** The predicted cell is put back by the
  existing block-correction path (`send_back_refused_edit`), the `Refused` row
  is retained, and ONE whole-window correction `Replace` follows to the live
  player (`HostedServer::queue_live_correction_replace`), never the teardown.
  Every in-flight action after it has an older base: `Superseded`, zero
  effect, its cell sent back (BRIDGE: the suffix is superseded, not replayed,
  Decision 16, until S3). Scope: typed edits (every Drift; a typed
  `Unsupported` edit, Q11, and an uncarriable yield, Q12, are refusals),
  personal clicks and a refused Autofill, shots, `OpenPlayer`/`OpenTable`.
  Still torn down (`refuse_typed_action_then_teardown`): container open and
  click refusals and the contest (no container view in a `Replace`),
  `SetAutoRefill` (a `Replace` keeps the refill setting), every U, C and A
  class. The live `Replace` takes no death flag (`replace_waiting` is the
  neutral flag; at most one waits) and must fit the pending window events
  (1024, 2 MiB) by its real owned heap (`strict_replace_fits`; run 12's red
  was `event_heap_bytes` refusing every `Replace` by design, so every probe
  "didn't fit"); one that doesn't is a `FellBehind` teardown. Per-seat
  `RefusalTally` (stale-world vs other) goes in the departure line, never as
  cheating. Tests: `hosted_server/c3d_refusal_correction_tests.rs`,
  `c3d_s1_correction_replace_past_the_pending_cap_tears_down`.
- **Client half not built.** No honest client sends `ActionIntent::Edit` yet:
  a Strict client's edits still ride its movement packet and fail closed. The
  client half reserves an Edit before predicting, as a click does; its receipt
  retires at the current boundary (a predicted edit numbers no event).

## Run 6 — every `Superseded` is corrected (coordinator rule after the run-5 review)

- **Rule.** A superseded action whose base predates the latest `Replace` is
  corrected by that `Replace`. A dead body's action on a current base gets
  one fresh whole-window correction `Replace` (`queue_correction_replace`,
  re-arming the death/join path); at most one is outstanding, so actions
  based before it are stale and supersede without another.
- **Every `Replace` carries the station and the lock mask** (`InventorySync`
  `station`, `locks`, v89). The server's copy installs the lock mask when it
  applies the `Replace`; the obligation fold takes the `Replace`'s locks and
  the raw copy's auto-refill (one source each, run-4 Nit). The client installs
  the lock mask and adopts the station (`CraftingUi::adopt_station`): a table
  screen the window no longer has closes locally, with no `Close` sent.
- **Client gate (flicker only).** In Strict, while the body is dead or a
  Respawn is unanswered (`GameState::strict_window_frozen`), Esc's
  `CloseCraft` and the table force-close send nothing. Not covered by an
  automated test (the GPU harness could drive it); the server rule is the
  guarantee and is tested.
- **Configuration at a stale base (run-5 LOW-2).** `stale_base` is computed
  for configuration ops too: one made before the latest `Replace` is served on
  the current copy without `apply_through`, and its digest is skipped exactly
  when its base predates the latest `Replace`.
- **Tests** (`hosted_server/c3d_correction_tests.rs`) model the client's window
  separately (shared click rule, `inventory_sync::apply`, `adopt_station`).

## Run 7 — Mirror sync unchanged (coordinator rule)

- **Only a Strict session applies the lock mask and station.** Run 6 made
  `inventory_sync::apply` install the `InventorySync` lock mask, and the game
  loop adopt its station, on every sync, Mirror included. Now
  `inventory_sync::apply` takes the session's `InventoryMode`: Strict installs
  the lock mask and returns the station to adopt; Mirror returns `None` and
  keeps its own locks and station exactly as on `bdd4fe87`. The packet still
  carries both in either mode.
- **Strict joiner saves don't keep locks** (accepted for now): persisting locks
  and auto-refill goes to SIDECar v3 with the fishing cooldown.
- **Test** `c3d_mirror_join_and_death_syncs_keep_client_locks_and_station`
  (real Mirror `HostedServer` join and death syncs; the same packets applied
  in Strict install both).

## Run 8 — the grid is never discarded, one client gate, the Replace station

- **H1 (run-6 review).** A `Replace` (join from a saved record, death,
  correction) can leave grid items behind a closed screen. Opening a crafting
  screen over a non-empty grid first sends a real `Close` at the current
  station, digested over the real grid (both copies return the grid), then
  opens; `reset_screen` no longer empties the grid; `open_joined` digests the
  real grid. A full bag that refuses the `Close` opens nothing new (the screen
  shows the leftovers at its own station). **Both modes:** in Mirror this
  deliberately fixes the pre-existing `bdd4fe87` bug (the open wiped a
  restored grid locally while the server's copy and the save kept it).
- **LOW-2.** `WindowEvent::Replace` carries the station (one send-time
  snapshot shared with `InventorySync.station`); a Strict copy's apply sets
  `sp.station`, so a configuration op served on a base before the `Replace`
  is overwritten on both copies. Mirror's server copy keeps its station.
- **LOW-1.** One gate, `GameState::strict_window_frozen`, read only inside
  `close_crafting` (which returns `CraftClose::{Closed, Refused, Frozen}`),
  `force_close_crafting` and an open's return-the-grid `Close`. The ad-hoc
  checks in the Esc chain and the table force-close are gone. Esc always takes
  the crafting arm when the screen is open (`GameState::esc_crafting`): frozen,
  it pauses, never the generic panel close. E's "Inventory full" toast shows
  for `Refused` only.
- **Accepted from run 7:** the server copy's `Replace` installs its own fold's
  locks in both modes (equivalent to the old behaviour in Mirror).
- **Tests** `hosted_server/c3d_grid_tests.rs` (a real `RemoteClient`, the real
  `CraftingUi` entry points; nothing forces the screen open) and the GPU
  harness `game_harness_strict_frozen_crafting_esc_pauses_and_e_returns_the_grid`.

## Run 9 — the client half of typed edits

- **The rule.** In a Strict session the client never puts a plain placement
  or a mined break on its movement packet. The game loop's place arm
  (`GameState::take_for_placement`) and break arm (`GameState::break_route`)
  route through `strict_edits::route_place` / `route_break`: the shared
  `judge` on the client's own window, then `sends_typed` (Strict and
  Place/Break). A typed edit reserves ONE numbered `ActionIntent::Edit`
  before prediction (recipe `EditRecipe { kind, slot, digest, sources }`,
  footprint = the cell), predicts with the shared window halves
  (`place_window`, `break_window`) the server stages on raw R, then sends.
  A break reserves (and wears its tool) before `set_block`; the arm skips
  `pending_block_changes` / `pending_mined` and the later wear. Mirror and
  unjoined worlds take exactly the old path.
- **Placement meta hoisted.** `placement_meta::placement_meta(block, face,
  forward)` is the one rule the place arm writes and a typed `Edit` carries
  (shape, piston/dispenser/dropper look, directional power axis; `None`
  keeps the cell's meta, as `World::set_block` does).
- **LOW-3.** A `Superseded` `Edit` calls `send_back_refused_edit` (its cell
  and a door's pair); `Edit` result rows (Accepted, Refused, Superseded)
  carry the edit's cell and a door's pair (`HostedServer::edit_cells`) via
  `prepare_typed_result_at` / `complete_typed_noop_at` /
  `refuse_typed_action_at`. The prediction pin is a test
  (`c3d_client_edit_prediction_is_the_server_staging`), not a wire digest.
- **Still paused in Strict (reach the movement packet, close the session):**
  creative edits, same-block edits, door place/break (the door top is a
  use-tagged edit), non-yielding manual breaks and every use-tagged edit.
  (The place arm only places into AIR or WATER, so the CLIENT never routes
  an edit to `judge`'s untagged non-emptying refusal. On the server it is
  reachable when the server's world is stale against the client's: another
  block now stands in the cell, so the received placement is refused and
  sent back. Run 11, run-9b review LOW-5.)
- **Left to recovery (recorded run 11, run-9b review LOW-2).** A Refused or
  Superseded placement's send-back restores the cell's block id and meta
  only. Nothing undoes what the client's place arm also wrote locally for
  the placement that never happened: `plots` (a Plot Marker's claim),
  `village_bells`, `salt_licks`; the Vendor, Tip Jar and Auction entity
  inserts; an opened sign editor. Mirror's refused path leaves the same.
  And `send_back_authoritative_block` returns silently for a column the
  server hasn't loaded, so a Superseded or Refused `Edit` there gets no
  world correction (narrow: the client still holds a column the server
  dropped). Both are recovery's job.
- **`Edit.source_tick` is reserved and unread.** The client stamps it (its
  next input sequence); the server never reads it. Nothing may depend on it
  until a decision gives it a meaning.
- **Tests** `hosted_server/c3d_client_edit_tests.rs` (a real `RemoteClient`,
  the client's window its own, routing through the production
  `route_place`/`route_break`) and the GPU harness
  `game_harness_strict_place_and_break_go_typed`.

## Run 13 — the run-12 review LOWs

- **LOW-1.** The live `Replace` reserves its deque slot before the byte and
  count bound is measured on the real capacity
  (`window_events::reserve_strict_replace`, the `prepare_strict_events`
  pattern: `try_reserve_exact`, measure, `shrink_to` on failure). Byte half
  pinned: `c3d_s1_correction_replace_past_the_byte_bound_tears_down`.
- **LOW-2.** `raw_marker` (and a waiting `Replace`'s window) returns
  `StrictStageError::RawUnrepresentable`; every Strict caller has its own arm:
  `HostedServer::refuse_raw_unrepresentable` (refused, then torn down,
  `StrictTeardown::Unrepresentable`). A result-side `Unrepresentable` (an
  action's grant, the edit clamp) stays a corrected refusal.
- **LOW-3.** `strict_admission::CorrectionRate` (BRIDGE until S3 replay):
  8 live correction `Replace`s per seat per 100 ticks; the ninth is a
  `FellBehind` teardown naming the bound.
- **LOW-5.** Only `judge`'s DRIFT counts as stale-world; a typed
  `Unsupported` edit and a break that doesn't yield that way count as rule
  refusals.
- **Container note.** A waiting container `Correction` makes a refusal's
  `Replace` a `FellBehind` teardown (fail-closed; `event_heap_bytes` has no
  measure for it).

## Run 13 — the client sends a typed shot

- `strict_requests::route_shot` (client): on a Strict seat, the auto-refill
  setting, then the `joiner_actions` record (the outcome's `seq` and the
  ammo claim), then ONE reserved `ActionIntent::Item(Shoot)` (recipe
  `ShotRecipe { slot, weapon, digest }`, no cells), then the send. No
  prediction: the server's Take and Wear arrive on the outcome. A failed
  reservation releases the record. Mirror: `ShotRoute::Legacy`, the old
  request unchanged (`GameState::send_shot`).
- Server: a shot whose payment the server's copy can't make
  (`StrictStageError::MissingPayment`) is a corrected refusal (it was a
  `FellBehind` teardown).
- No GPU harness shot rig exists (`test_game_harness.rs` has none); the
  client half is pinned on a real `RemoteClient`
  (`hosted_server/c3d_client_shot_tests.rs`).

## Run 14 — the run-13 review LOWs

- **LOW-1:** `c3d_s1_reserved_replace_never_grows_the_deque_past_its_measure` pins the reserve-before-measure fix: a deque full to its capacity grows on the reservation, and the `push_back` that queues the `Replace` never grows it further. Control (measure without reserving) is red.
- **LOW-2:** `RawUnrepresentable` is pinned through the edit arm (`c3d_s1_raw_unrepresentable_placement_is_refused_then_torn_down`) and the shot arm (`c3d_client_strict_shot_on_a_raw_unrepresentable_window_is_refused_then_torn_down`), not only the configuration arm. Routing those arms to the correction is red.
- **LOW-3:** the dead-body correction `Replace` (`queue_correction_replace`) goes through the same `admit_correction_replace` as the live one: one 8-per-100-ticks bound per seat and the reserved fit (`c3d_s1_dead_body_corrections_share_the_rate_bound`).
- **LOW-4 (decided):** "the server's cell doesn't break that way" counts `StaleWorld` when the server's cell (block and meta) differs from the client's `old`, else `Rule` (two tests in `c3d_edit_tests.rs`).
- **LOW-5:** `strict_bridge_teardown` records its detail on the test thread (`actions::teardown_detail`); every test that asserts the `FellBehind` sentence also asserts its internal detail (`CORRECTION_RATE_DETAIL`, `CORRECTION_FIT_DETAIL`, or the path's own).

## Run 14 — a refused `SetAutoRefill` is corrected

A Strict `SetAutoRefill` the server's copy can't reproduce (its digest) is a typed refusal with zero effect and one correction `Replace` inside the rate bound, as `OpenPlayer`/`OpenTable` are; a raw-unrepresentable window still tears down with its own sentence. The `Replace` keeps each copy's refill setting, so convergence needs the client: when a `Refused` receipt for its `SetAutoRefill` retires, `RemoteClient::refill_refused` is set, and `CraftingUi::sync_auto_refill_joined` forgets that the setting was sent (`WindowOpLog::forget_auto_refill`) and sends it again on the corrected window, where it is served. Mirror never reserves a setting, so it is unchanged. Test `c3d_s1_refused_auto_refill_setting_is_corrected_then_resent` (revises run 13's teardown pin); restoring the teardown is red ("corrected, never torn down").

## Run 15 — the resend bound, and the typed entity attack

- **Resend bound:** a `SetAutoRefill` refused on every resend (the server's copy diverging again each time) is corrected once per refusal, one resend per correction, and the ninth correction inside 100 ticks is the rate bound's `FellBehind` teardown (`CORRECTION_RATE_DETAIL`), with zero item effect (`c3d_s1_a_refill_resend_refused_again_ends_at_the_rate_bound`). The honest case is served on the first resend (run 14's test).
- **Typed attack (`hosted_server/strict_attack.rs`, `serve_strict_attack`).** The Mirror swing is split into the shared rule `HostedServer::check_joiner_swing` (target alive, in reach and ahead of the server's body, no friendly fire unless sneaking, the swing schedule; no effect) and `strike_joiner_swing` (schedule move, `combat::strike`, `after_swing`, the kick-back). Mirror runs both exactly as before, damage from the claim (BRIDGE). A Strict seat's `ActionIntent::Attack`: hotbar slot ≥ 9 refuses (no latest-slot fallback); the weapon is the server copy's item in the named slot at the action's boundary, and a claim that disagrees is a corrected `Rule` refusal (the shot's rule; an empty hand is a valid claim). A swing the shared rule refuses is the same corrected refusal: zero external effect (no damage, knockback, `LastAttacker`, schedule move, wear or event). A tool's wear is ONE `WearWeapon` event, staged and reserved with the result row BEFORE the damage lands; a bare hand or a non-tool item wears nothing. The damage is the effective surviving item's; a pending wear that broke it is a `FellBehind` teardown (shot parity). Caps (1024 events, 2 MiB, the result history) leave zero effect and tear down, as a shot's. The answer is Mirror's `InteractOutcome` naming the Wear, applied by the client's existing `joiner_actions::apply_outcome`. After the commit, `strict_fold_holds`. A dead attacker's action is `Superseded` (with its correction) before the arm runs.
- **Client:** `strict_requests::route_attack` (from `GameState::send_entity_attack`): the refill setting, the `joiner_actions` record, ONE reserved `ActionIntent::Attack` (recipe `AttackRecipe { slot, digest }`, no cells), the send; nothing predicted. Mirror sends the legacy `EntityAttack` request unchanged.
- **Known costs (flagged, not changed):** every refused swing (out of reach, cooldown, target died in flight) draws a whole-window correction `Replace` and one tick of the 8-per-100 rate bound, though the client predicted nothing; melee makes that more reachable for an honest player than a shot. A sword on its last point swung twice inside one round trip is a `FellBehind` teardown (the shot's survivor rule).
- Tests: `hosted_server/c3d_client_attack_tests.rs`.

## Run 15 — the run-14 review LOWs (mid-run addition)

- **LOW-2 (revises run 14's LOW-4 decision):** "the server's cell doesn't break that way" is `StaleWorld` only when the server's block id differs from the client's `old` (a real Drift); a meta-only difference is `Rule` (`c3d_strict_doesnt_break_that_way_on_a_meta_only_difference_is_a_rule_refusal`). `RefusalTally`'s doc records that a modified client can steer the bucket (a forged `old` id is a Drift); classifying from server-side facts waits for S2/S3's capture.
- **LOW-3:** every literal `FellBehind` detail is distinct (`c3d_fell_behind_details_are_distinct` scans the Strict server sources); a whole event reservation names its cap (`actions::reservation_detail`: the pending-event count or byte cap), asserted by the shot and attack cap tests.
- **LOW-4 (checked):** the raw-unrepresentable placement's send-back is queued but never reaches the client: the teardown frees the seat before the tick's block broadcast (harmless: the client leaves the world). Recorded in the test. The split-pump refill test and **LOW-1** (a click in the gap between the correction `Replace` and the `Refused` receipt) are open for the next run.

## Run 16 — honest attacks never cost a correction; the named-slot Wear

- **World-fact refusals (`HostedServer::refuse_world_fact`, `hosted_server/actions.rs`).** A Strict swing the shared Mirror rule (`check_joiner_swing`) refuses after a MATCHING claim (out of reach or facing on the server's body, the swing schedule, a target gone or dead, its own pet) is a `Refused` row alone: no correction `Replace`, no tick of `CorrectionRate`, nothing superseded behind it. Precondition, asserted in code: `ActionIntent::predicts_nothing` (true only for `Attack`), read by the client's `route_attack` (`debug_assert`) and required by the server helper (any other intent falls back to `refuse_typed_action`). Counted in `RefusalTally::world_fact`, apart from Drift and rule/mismatch. Budget: the Strict queue's `MAX_ITEM_ACTIONS_PER_TICK` (4 per tick, shared with shots); past it the head waits. A window disagreement (claim ≠ the server copy's item, slot ≥ 9) stays a corrected `Rule` refusal. A dead attacker stays `Superseded` before the arm.
- **Survivor rule (attack and shot).** A pending Wear the client hasn't applied that broke or changed the claimed weapon is `refuse_typed_action` (one `Replace` after the Wear), no longer a `FellBehind` teardown. Cannot loop: the next use names an empty hand or a different item.
- **Named-slot Wear (`joiner_actions::strict_wear_target`, used by `window_events::strict::apply_marker_event`).** The hint slot when it holds the kind; else the unique match; else `AmbiguousWear`. Every `Some` equals `where_now`'s pick, so both copies wear the same piece.
- **Run-14 review LOW-1.** `RemoteClient::poll` sets `refill_refused` when a Strict `InventorySync` arrives while a `SetAutoRefill` is unretired; the receipt backstop fires only when no later setting is journaled.
- Tests: `c3d_client_attack_tests.rs` (out-of-reach uncorrected, nine misses one per cooldown, early swing, dead body, last-point twice), `c3d_client_shot_tests.rs` (last-point bow twice), `c3d_wear_target_tests.rs` (spare identical sword / bow), `c3d_refusal_correction_tests.rs` (click between Replace and receipt, split pump), `test_game_harness.rs` `game_harness_strict_joiners_swing_is_served_typed` (GPU).

## Run 17 — the run-15 review LOWs

- **LOW-1 (evidence):** the "a claim refusal applies knockback" control re-run alone, failure line captured: `no knockback (80.3822 vs 80)` (`c3d_client_strict_claimed_better_sword_is_refused_and_corrected`). It proves the claim gate: `claim_matches` requires full equality, so removing the gate is the only route by which a claim could reach the effect.
- **LOW-2:** a `Refused` or `Superseded` receipt for a typed `Attack` or `Item(Shoot)` releases the request's `joiner_actions` record when it retires (`RemoteClient::released_requests`, bounded by `MAX_PENDING`, drained by `GameState::release_retired_requests`): no outcome will answer it, so a refused shot's ammo stops claiming at once and stale records no longer crowd the 64 slots. World-fact refusals are `Refused` rows and are covered.
- **LOW-3:** `c3d_client_strict_attacker_killed_by_the_nostrich_kick_back_conserves_the_worn_sword`: a 1 HP Strict joiner swings an iron sword at a wild Nostrich; the swing is Accepted (the fold holds), the kick-back kills it, the death `Replace` is numbered after the Wear, and the sword, worn exactly once, is conserved over window, graves and ground.
- **LOW-4:** `c3d_fell_behind_details_are_distinct` now classifies every `FellBehind` argument (literal, `strict_raw_or_stop` path, `reservation_detail`, the two correction consts; anything else fails) and checks all of them distinct, both reservation texts included. A non-count reservation failure is labelled "byte cap or allocation" (`actions::reservation_detail_text`).

## Run 17 — the run-16 review (mid-run addition)

- **LOW-1 (decided):** `ActionIntent::predicts_nothing` now covers `Item(Shoot)`; `serve_strict_shot`'s schedule refusal goes through `refuse_world_fact` (receipt alone, no `Replace`, counted `world_fact`); ammo/bow missing from the server copy stays corrected. The tautological `debug_assert` in `route_attack` is gone; the table is backed by `strict_requests::NoPrediction`, taken by the game loop before the joined swing branch (through the local-hit wear block) and around `send_shot`, checked after: a digest change is `log::error!` plus `debug_assert!`. No window event is applied inside those branches. Tests: `c3d_client_strict_early_shot_is_refused_uncorrected`, `c3d_strict_no_prediction_guard_trips_on_a_predicted_swing_wear`, `c3d_strict_no_prediction_guard_trips_on_a_predicted_shot`.
- **LOW-2 (decided, pinned, no behaviour change):** `c3d_strict_last_point_sword_replaced_in_flight_by_an_identical_kind_is_a_known_teardown` — an honest teardown (`FellBehind`, "Attack wear or pending fit is uncertain"), documented in Spec 04 until S3 replay.
- **LOW-3:** run-15 L2 (`released_requests`), above.
- **LOW-4 (documented, no change):** `RemoteClient::poll` sets `refill_refused` on any Strict `InventorySync` while a `SetAutoRefill` is unretired, including one already served; the resend of an unchanged setting is served harmlessly, at the cost of one extra journaled `SetAutoRefill` per correction or death sync that lands while a setting is in flight.
- **LOW-5:** an action right behind a world-fact refusal in the same round trip is served (`c3d_client_strict_action_behind_a_world_fact_refusal_is_served`); own-pet refusal on a signed-in seat (`c3d_client_strict_swing_at_own_pet_is_refused_uncorrected`); `strict_wear_target`'s unique fallback pinned (`c3d_wear_target_falls_back_to_the_unique_piece_elsewhere`); the dead-body test's knockback assert compares with `y0`.
- Controls (`/tmp/c3d-run17-ctrl1.log`, one build, three disjoint mutations, SHA-256 identical after restore): early shot corrected → "the first's Take and Wear, no correction Replace left: 3 right: 2"; guard always `Ok` → "a predicted wear trips the guard: ()" and "a predicted ammo take trips the guard"; unique fallback → `None` → "the unique piece elsewhere left: None right: Some(20)".

## Run 18 — doors (Decision 20)

- Item 0 (finding): a door/gate/trapdoor toggle is client-local in every
  mode; a Mirror joiner's (or a host's) toggle never reaches the server or
  the other players. Pinned, ignored: `c3d_mirror_joiner_door_toggle_reaches_the_server`.
  Mirror unchanged; the coordinator decides its fix.
- The toggle footprint moved out of `game_loop.rs` into
  `strict_doors::toggle_footprint` / `toggle_metas`: Mirror `FlipEach`
  (byte-for-byte the old arm, nine layouts), Strict `Canonical`.
- Strict door placement = ONE `DoorPlace` (two credits, one door, both
  halves and one row together; no `DoorUpper`); door break = typed `Edit`
  Break (two credits, pair permission preflighted, pair derived once);
  toggle = typed `ToggleBlock` (every footprint cell's permission, reach to
  the primary by the bare-hand rule). All refusals corrected (cells sent
  back, `Refused` row, one `Replace`); a superseded door action sends its
  cells back. Server: `hosted_server/strict_door.rs`. Spec 04 `0x44` "Doors".

## Run 19 — doors open for everyone (run 18's Mirror finding fixed)

- The one sanctioned Mirror change (coordinator, 2026-10-12): the door
  toggle path only. A Mirror joiner's door/gate/trapdoor click is a typed
  `ToggleBlock` (`strict_doors::reserve_toggle`, no refill setting), still
  predicted locally by `FlipEach`. The server arm is shared
  (`HostedServer::serve_toggle`, dispatched by `strict_slot` or
  `mirror_slot`): Strict = `Canonical` + corrected refusals; Mirror =
  `FlipEach`, refusals sent back with a `Refused` row and no `Replace`; a
  superseded Mirror toggle sends its cells back with no `Replace`.
- Validation in both modes = a joiner's block edit: the primary through
  `validate_block_edit` (reach with the server copy's item in the named slot,
  `remote_may_touch`, economy owner), the companions through `cell_permitted`.
- Wire (v89 unreleased): `ToggleBlock { cell, expected_block, hotbar_slot }`,
  layout pinned (`c3d_wire_toggle_block_carries_its_hotbar_slot`). Run 18's
  Reach Claw note is closed.
- Host/single-player: the toggle arm records each footprint cell on
  `pending_block_changes` (`broadcast_change`) when not joined; a lending
  host broadcasts it unvalidated like its other edits; single-player discards
  it at the send. `--no-lend`: by reading, the owning server applies the
  host's meta-only edits through the local-slot path (budget 4/tick, refused
  cells sent back); untested, left under the `--no-lend` BRIDGE.
- Mirror behaviour change to know (superseded by run 20): a Mirror joiner in Adventure/Spectator
  (`can_edit_world` false) can no longer keep a door open on the server (its
  predicted door is sent back shut); before, it opened only on its own screen.

## Run 20 — a door toggle is a use, not an edit

- Coordinator decision (2026-10-12): a toggle's play-mode rule is exactly
  the single-player toggle arm's (no gate on main), so Adventure and
  Spectator joiners open doors, in Mirror and Strict, and every player sees
  it. One rule, `strict_doors::toggle_mode_allows` (exhaustive), asked by
  the client preflight (`toggle_preflight`) and the server
  (`HostedServer::toggle_cell_permitted`, used for the primary and every
  companion in place of `validate_block_edit` / `cell_permitted`). The
  server's other checks are unchanged and in an edit's order: in the world,
  loaded, not bedrock, reach with the server copy's slot item (primary
  only), foreign plot (`remote_plot_allows`, split out of
  `remote_may_touch`, whose other callers are unchanged), economy owner.
- Owner question: should Spectator toggle at all? Kept as single-player has
  it.
- Run-19 review HIGH-1 + MEDIUM-1: a Mirror seat's `ToggleBlock` is
  window-free (`actions.rs` `window_free`): no boundary check, no
  `apply_through`, never stale on a `Replace` (served; superseded only on a
  dead body); a Mirror seat's `Action` packet waits in inbound behind that
  joiner's waiting edits (`hosted_server.rs` read loop,
  `is_typed_action`). Residue: a toggle already deferred on the item budget
  can run after later edits (world only, refused + sent back if stale).
- Run-19 review LOWs: LOW-1 checked neighbour arithmetic in
  `toggle_sendback_cells`; LOW-2 no correction `Replace` for a superseded
  toggle (either mode); LOW-3 the held-click test paced (25 clicks / 200
  ticks, `CorrectionRate` untouched); LOW-4 an accepted toggle sends back
  the union less its footprint (ghost companions); LOW-5 notes in Spec 04.

**Run 21 (2026-10-12), run-18 review L1, L3–L6.** The `NoPrediction` bracket guard now also requires each close to precede the next bracket's open and to check its own guard. A Strict door break journals both predicted cells (the union with the server row's live pair is the S3 footprint). A joined client refuses a door whose top would stand above the world (before, a Mirror client placed a bottom-only half door and sent a `DoorUpper` coupon for a cell outside the world, and a Strict client's `DoorPlace` was a corrected `OutOfWorld` refusal per click; run-22 review LOW-4). A creative world's `DoorPlace` is a corrected `Rule` refusal. A dead body's `DoorPlace` is superseded whole. The game loop's door wiring is pinned by source position.

**Run 21, run-20 review LOW-1..6.** An accepted door click's ghost send-backs (at most 14, broadcast to every joiner, from the clicker's 64 a tick) are documented in Spec 04. A Mirror joiner's door click waited while its own edits were unsent (toast, nothing sent or predicted) — superseded in run 22. A Mirror toggle made before the join sync's `Replace` is applied is served (tested). The `--no-lend` host's brief door flip-back and the kiosk's shared door state are noted in Spec 01 and Spec 05. The HIGH-1/MEDIUM-1 controls were re-run with logs kept (`.claude/epic/logs/c3d-run21/`).

**Run 22 (2026-10-12), a door click is never refused for waiting edits.** A Mirror joiner's door click is predicted at once and its reserved `ToggleBlock` joins the client's one ordered queue (`Request::Action`): sent now, or right after the input carrying the edits made before it. Run 21's refuse-and-toast rule (which lost the click) is gone; the server's MEDIUM-1 hold stays.

## Run 23 — typed entity interactions

- **Typed interactions (`hosted_server/strict_interact.rs`, `serve_strict_interact`).** A Strict seat's `ActionIntent::Interact` — `Feed`, `Tame`, `Shear`, `Milk`, `LeadAttach`, `LeadDetach`, `SitToggle`, `LeadToPost`, `Mount` — is served instead of paused `Untyped` (the legacy `EntityInteract` still tears a Strict seat down). Order: slot ≥ 9 or a claim that disagrees with the server copy's item in the named slot → corrected `Rule` refusal (`strict_attack::claim_parts_match`); the pure world-fact gate (`interaction_world_fact`: target / cart / post reach, `NotOnThisServer`, already riding, cooldown) → `Refused` row alone plus Mirror's `InteractOutcome { accepted: false, note }`; the cost from the pure table `mob_interact::accepted_cost` staged on the effective window (`Take` from the named slot, then `Grant`s; a payment the window can't make, or a consume-free tool a pending event moved, → corrected); events + result row reserved; then Mirror's run with the server copy's item (`run_joiner_interaction_with`, which Mirror's `run_joiner_interaction` now calls with the claim — byte for byte). A refusal inside the run abandons the reservation exactly (the cooldown it starts is Mirror's too) and is the world-fact refusal; an accepted run whose cost differs from the table is a fail-closed `FellBehind` pause. Accepted: commit, `Accepted` row, outcome naming the `Take`, grants, `strict_fold_holds`.
- **Client.** `GameState::send_interaction` (feed … mount, lead to a post) routes a Strict seat through `strict_requests::route_interact` (recipe `InteractRecipe { slot, digest }`, no cells) inside a `NoPrediction` "interaction" bracket (pinned by the bracket-position guard); Mirror sends the legacy request unchanged. `ActionIntent::Interact` joins `predicts_nothing`; `unanswered_request` releases a refused or superseded interaction's request record.
- **Tests:** `c3d_interact_tests.rs` (each kind served with no teardown; a claim the copy lacks → corrected; `NotOnThisServer` → note + receipt, uncorrected; nine out-of-reach clicks in < 100 ticks → no teardown; Mirror stays legacy), `mob_interact::tests::c3d_accepted_cost_is_pinned_against_the_runs`.
- **Run-22 review LOW-1:** a Mirror seat's inbound loop now waits while a typed action (a toggle deferred past the item budget) waits in its action queue (`HostedServer::typed_action_waits`), so a later input never overtakes the click (`c3d_mirror_deferred_toggle_is_not_overtaken_by_a_later_input`); LOW-4: the run-21 paragraph's ceiling reason corrected.
- **Still open (at run 23):** eating (`ItemAction::Eat`), `DeviceInteract`, creative uses — typed in run 24, below.

## Run 24 — eating, power devices and creative gives

- **Typed (`hosted_server/strict_item.rs`).** A Strict seat's `ActionIntent::Item` carrying `Eat` or `CreativeGive`, and `ActionIntent::Device { cell, hotbar_slot }`, are served instead of paused `Untyped`. The client predicts nothing for any of them (an eat's only local change is the eat cooldown that paces clicks; a give lands by its numbered `InventoryGrant`; a device flips by the broadcast), so all three join `ActionIntent::predicts_nothing`: world facts are a `Refused` row alone, with Mirror's `ItemActionOutcome { accepted: false, note }` for an eat or a give.
- **Eat:** the food from the server copy's named slot (slot ≥ 9, a disagreeing claim, or not food there → corrected `Rule`); `judge_eat` (not hungry, eating cooldown) → world fact; else `Take` 1 staged and reserved with the result row, then `item_actions::eat` and the cooldown.
- **Creative give:** not a Creative world (`NotHere`) or no room in the effective bag (`InventoryFull`) → world fact (a held give on a full bag never draws on the correction bound); unreadable stack or a Plan stack ≠ 1 → corrected `Rule`; else one `Grant` (a Plan as its private marker) staged, reserved, sent numbered, then the outcome.
- **Device:** Mirror's rules (`remote_may_touch`, reach, a toggle-class device) refused as world facts, with reach from the server copy's item in the named slot (a Reach Claw counts there; Mirror's flat device reach unchanged); then `power::interact_device` on the server's world, `Accepted` row carrying the cell.
- **Client:** `strict_requests::route_item` (eat, give; `GameState::route_item_request`) and `route_device`, each inside a `NoPrediction` bracket ("eat", "creative give", "device"), pinned by the bracket-position guard. `unanswered_request` releases a refused eat or give. Mirror sends the legacy packets byte for byte.
- **Tests:** `hosted_server/c3d_item_tests.rs` — eat served; eat claim the copy lacks → corrected; nine eats on a full stomach → world facts, no teardown; give served; give in survival / into a full bag → uncorrected; device served; device reach from the named slot (claimed claw refused, copy's claw served); Mirror stays legacy; a Strict seat sending a legacy `DeviceInteract`, `ItemAction` (eat, drop) or `EntityInteract` is torn down with the `Untyped` reason.
- **Still legacy on a Strict seat (fail closed, torn down `Untyped` by `reject_legacy_inventory`, never silently served):** the movement packet's use-tagged edits (the eleven standalone uses: bucket fill/empty, sow, papyrus, bone meal ×2, salt, erase, tap rubber, till, hang print), and the `ItemAction`s `Sleep`, `Drop`, `GrantUnfit`, `UseBlock` (and the campfire light), `PlaceCart`, `Cast`, `Reel`, `PlanMinted`, `Attach`, `Detach`, `OpenGrave` (`Craft` is ignored on every seat). `HoldBegin/Cancel/Strike` have no client sender yet (a typed one pauses `Untyped`). `Respawn`, `ChatSay`, the applied acknowledgement and the movement packet without edits carry no inventory or world authority.
- **Run-23 review (run 24).** MEDIUM-1: the moved-item check in `strict_interact.rs` covers `Shear` only and compares the tool's type and material (not the tier `ItemRef`); a mount or Lead removal right after a feed took the hand's last unit is served (`c3d_client_strict_mount_after_a_feed_took_the_last_unit_is_served`, which is also the steed mount's end-to-end cost pin with an item in hand; the cart-with-freight branch stays pinned by the runtime fail-closed check only). LOW-1: the in-run refusal path is tested (`c3d_client_strict_feed_refused_inside_the_run_abandons_the_reservation`: an in-love cow, `last_number`, waiting count and capacity unchanged, food on both copies). LOW-2: Spec 04 and the module doc say what the rule covers. LOW-3: a Strict mount reads `MountTaken` before the cooldown (`c3d_client_strict_mount_taken_inside_the_cooldown_keeps_mirrors_note`).

## Run 25 — Q-drop and sleep

- **Drop (`hosted_server/strict_drop.rs`).** The one predicted item action: the client reserves ONE typed `ItemAction::Drop` (`strict_requests::route_drop`, recipe `DropRecipe { slot, digest-after, sources }`) before its hand's `take_one_from_hotbar`, so every refusal is corrected. The server copy's named slot pays: slot ≥ 9, a disagreeing claim, an empty slot or a Plan → corrected `Rule`; the take staged by `stage_strict_edit` (no numbered event), result row, commit, a drop-bucket token, the copy's real item thrown from the server's body. Mirror's believed-units bound is never reached from Strict. The drop bucket DEFERS a typed drop at the queue head, never refuses it.
- **Sleep (`strict_item.rs`, `serve_strict_sleep`).** Claims and predicts nothing (`predicts_nothing`; the "sleep" `NoPrediction` bracket): Mirror's `judge_sleep` facts → note + `Refused` row, uncorrected; else result row, `serve_sleep` (spawn, heal), accepted outcome. No item, so no "claim the copy lacks" case exists.
- **Tests:** `hosted_server/c3d_drop_sleep_tests.rs` — drop served from the copy; drop claim the copy lacks → corrected; nine quick drops wait for the bucket, all served; sleep sets spawn and heals; nine noon sleeps → world facts, no `Replace`; Mirror's drop stays legacy.
