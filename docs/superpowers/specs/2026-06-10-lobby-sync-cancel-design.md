# Lobby "Sync Stash" — wait-for-bunker + Cancel + Messages label — design

**Status:** APPROVED (brainstorm 2026-06-10) — proceed to plan + build.
**Scope:** the **lobby** Sync Stash button only (`menu.rs` main-menu / world-list). The in-game pause-menu "Save & Stash" is unchanged.

---

## 1. Problem / goal

Today's lobby Sync Stash (`kick_off_stash_sync`, `menu.rs`):
- **Bails instantly** if the bunker is off ("Cloud signer not connected — open your bunker in Signet, then Sync") — there's nothing to cancel, and you must open the bunker *before* tapping.
- **Flushes queued `/bug`·`/idea` messages only as a side effect of a world upload** — so if you have messages queued but no pending worlds, they don't send.
- Fire-and-forget; no in-progress/cancel state.

Goal: tap Sync Stash, *then* open your bunker on the phone — it **waits**, connects when the bunker opens, backs up worlds, **and always sends queued messages**. A **Cancel** escape hatch and a timeout keep it from hanging. The label shows queued-message count so a kid sees their reports are about to send.

## 2. Agreed behaviour (from brainstorm)

- **Wait-for-bunker (option A):** tap Sync Stash → button flips to **"Cancel Syncing Stash"** and waits; the moment you open your bunker it connects and finishes on its own. Cancel is the escape hatch; a ~90s timeout bounds the wait.
- **Dynamic label (option A):** Idle reads **"📦 Sync Stash"** with nothing queued, **"📦 Sync Stash & Messages (N)"** with N reports queued. *(Future: when player↔player DMing ships, flip to always-on "Sync Stash & Messages" — drop the count-gating. Noted so we don't forget.)*
- **Messages always flush** — even with zero pending worlds.
- **Lobby only.**

## 3. Architecture

Move the batch **orchestration into JS** (one abortable async function); keep `menu.rs` as a thin **UI state machine** that renders the button and polls status. The wait / abort / timeout / flush is Promise + `AbortController` work that belongs in JS; Rust stays simple. The current Rust upload loop (`kick_off_stash_sync`) is replaced by the JS batch + a Rust poll.

### 3.1 JS — the batch (`cloud.js`, new section)

Module state: `let _syncAbort = null; let _syncStatus = { state: 'idle', message: '', done: 0, total: 0 };`

- **`window.axenstax_sync_stash_start(pubkey, worldsJson)`** — `worlds = JSON.parse(worldsJson)` (array of `{folder, display}`); `_syncAbort = new AbortController()`; set `_syncStatus = {state:'syncing', message:'Open your bunker in Signet…', done:0, total:worlds.length}`; kick off `runBatch(pubkey, worlds, _syncAbort.signal)` (not awaited) which writes the terminal status into `_syncStatus`. (`pubkey` is needed for `axenstax_load_world`; Rust already holds it in `WASM_PUBKEY`.)
- **`window.axenstax_sync_stash_cancel()`** — `_syncAbort?.abort()`.
- **`window.axenstax_sync_stash_status()`** — returns `JSON.stringify(_syncStatus)`.

`runBatch(pubkey, worlds, signal)`:
1. **Wait for a capable signer** — `signer = await waitForCapableSigner(signal, SYNC_WAIT_TIMEOUT_MS)` (≈90_000). This retries reconnecting the **already-paired** bunker (`restoreSession({reconnectBunker:true})`) every ~2s until `capable()`, the signal aborts, or the timeout elapses. **It does NOT call `Signet.login()`** — no login-modal popups during the wait; first-time pairing is a sign-in concern, not this batch. Returns the signer, or null on abort/timeout.
   - aborted → `{state:'cancelled', message:'Sync cancelled.'}`
   - null (timeout) → `{state:'error', message:"Couldn't reach your bunker — open it in Signet and tap Sync Stash again."}`
2. **Back up each pending world** — for each `{folder, display}`: if `signal.aborted` → return `{state:'cancelled', message:'Cancelled — backed up '+ok+' of '+total+'.'}`; update `_syncStatus.message = 'Backing up '+display+'…'`, `.done = ok`; `blob = await window.axenstax_load_world(pubkey, folder)`; `if (blob && await AxeCloud.save(display, blob)) ok++`.
3. **Flush queued messages — always** — `try { const r = await mailboxFlush(); sent = r.sent||0 } catch {}`. (Signer already warm from step 1, so no extra prompt.)
4. **Result** — `{state:'done', message: resultText(ok, total, sent)}`.

`waitForCapableSigner(signal, timeoutMs)` — new helper near `ensureCapableSigner`:
```
if (capable()) return rawSigner();
const deadline = Date.now() + timeoutMs;
while (Date.now() < deadline) {
  if (signal.aborted) return null;
  try {
    const r = await window.Signet.restoreSession({ reconnectBunker: true });
    if (r && r.signer) { _setSigner(r.signer); if (capable()) return rawSigner(); }
  } catch (e) { /* keep retrying — bunker may not be open yet */ }
  await abortableSleep(2000, signal);   // resolves early if aborted
}
return null;
```

### 3.2 JS — message count for the label (`cloud.js`)

A cheap **synchronous** read for the per-frame label, kept fresh in JS (avoids an async round-trip every frame):
- `let _unsentCached = 0;` `async function refreshUnsent(){ try { _unsentCached = (await window.AxeMailbox?.unsentCount?.()) || 0 } catch {} }`
- Call `refreshUnsent()` on load, after each `__axenstax_mailbox_enqueue`, and after each `mailboxFlush`.
- **`window.axenstax_mailbox_unsent()`** → returns `_unsentCached` (sync).

### 3.3 Rust — the UI state machine (`menu.rs`)

Replace `stash_sync: Option<Rc<RefCell<Option<String>>>>` with a simple flag + reuse `stash_sync_msg`:
- `pub stash_syncing: bool` (true between Start and a terminal status).
- `pub stash_sync_msg: Option<String>` (live + final status line — unchanged field, reused).
- `pub unsent_count: u32` (cached label count, refreshed from `axenstax_mailbox_unsent()`).

Pure, unit-tested helpers:
```rust
/// Idle: "📦 Sync Stash" or "📦 Sync Stash & Messages (N)"; syncing: "Cancel Syncing Stash".
pub fn sync_button_label(syncing: bool, unsent: u32) -> String {
    if syncing { "Cancel Syncing Stash".to_string() }
    else if unsent > 0 { format!("📦 Sync Stash & Messages ({unsent})") }
    else { "📦 Sync Stash".to_string() }
}
/// True if a click should CANCEL (vs START).
pub fn sync_button_is_cancel(syncing: bool) -> bool { syncing }
```

Button (replaces the fixed `"📦 Sync Stash"` block at ~1413):
- label = `sync_button_label(state.stash_syncing, state.unsent_count)`.
- on click: if `sync_button_is_cancel(state.stash_syncing)` → `axenstax_sync_stash_cancel()`; else compute `pending` (unchanged filter) → `axenstax_sync_stash_start(pubkey, json)`, `state.stash_syncing = true`, `state.stash_sync_msg = Some("Opening your bunker…")`.

Poll (in `poll_cloud_worlds`, replacing the slot drain at ~432): parse `axenstax_sync_stash_status()`:
- update `stash_sync_msg = status.message`.
- if `status.state` is terminal (`done`/`cancelled`/`error`) and `stash_syncing` was true → `stash_syncing = false`; on `done` → `mark_cloud_dirty()` (flip greens).
- refresh `unsent_count` from `axenstax_mailbox_unsent()` (cheap sync bridge) here too.

### 3.4 Bridges (Rust extern → JS)

`menu.rs` (or `wasm_save.rs`) extern block:
- `axenstax_sync_stash_start(pubkey: String, worlds_json: String)` · `axenstax_sync_stash_cancel()` · `axenstax_sync_stash_status() -> String` · `axenstax_mailbox_unsent() -> u32`.

## 4. Result text (kid-friendly)

`resultText(ok, total, sent)`:
- `total==0 && sent==0` → "All backed up — nothing to sync."
- `total==0 && sent>0` → `Sent ${sent} message(s).`
- `ok==total && sent==0` → `Backed up ${ok} world(s).`
- `ok==total && sent>0` → `Backed up ${ok} world(s) and sent ${sent} message(s).`
- `ok<total` → `Backed up ${ok} of ${total} — reopen your bunker for the rest.`
- cancelled → "Sync cancelled." / mid-batch cancel → "Cancelled — backed up N of T."
- timeout → "Couldn't reach your bunker — open it in Signet and tap Sync Stash again."

## 5. Cancel + timeout invariants

- Cancel aborts instantly whether **waiting** for the bunker or **mid-upload**.
- Timeout (≈90s) ends the wait with the actionable "open it in Signet and tap again" message.
- Neither ever clears the bunker pairing (signet-login keeps creds on reconnect failure) nor un-uploads already-backed-up worlds.

## 6. Testing

- **Rust (unit, in `check.sh`):** `sync_button_label` (idle 0 / idle N / syncing) and `sync_button_is_cancel`. Cross-platform pure fns.
- **JS:** best-effort — `resultText` is a pure function (unit-test under the mailbox harness if wired; else reasoned + live-verified). The abortable wait loop is verified live (tap Sync, open bunker, observe connect; tap Cancel, observe instant return).
- **Live (owner):** tap Sync Stash with bunker off → "Cancel Syncing Stash"; open bunker → completes; queue a `/bug`, see "(1)" on the label, sync, see "sent 1 message".

## 7. Out of scope

- In-game pause-menu "Save & Stash" (unchanged; already warms a signer on demand).
- First-time bunker **pairing** from the lobby wait (the wait only *reconnects* an existing pairing; pairing happens at sign-in / in-game).
- The always-on "Sync Stash & Messages" label (lands with player↔player DMing — future).
