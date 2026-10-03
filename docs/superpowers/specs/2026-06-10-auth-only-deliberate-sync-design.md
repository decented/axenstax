# Auth-only sign-in + deliberate cloud Sync — design

**Status:** DRAFT — pending owner review, then implementation plan.
**Date:** 2026-06-10
**Owner-facing name:** "Save = this device. Sync = the cloud."
**Supersedes (behaviour):** the always-on background bunker reconnect at game load
(`auth.js`) and the auto-stash-on-every-save coupling.

---

## 1. Problem

On the live PWA, cloud save ("Stash") and the `/bug` · `/idea` feedback flush both
gate on `capable()` — a Signet **bunker** signer with live NIP-44 + `signEvent`
(`cloud.js:39`). Two things make that gate unreliable during play:

1. **Background reconnect race at load.** A returning user's bunker is reconnected
   *in the background* after `/game` boots (`auth.js:416–440`). A phone bunker can
   take >15s, or never land.
2. **Phone bunker auto-locks on backgrounding.** Once the player tabs away or the
   phone sleeps, the bunker clears its encryption key, so `capable()` flips back to
   false mid-session. Saves then show "📦 Couldn't stash — saved on this device"
   and queued feedback reports never flush.

Net: stashing (and therefore feedback delivery) "doesn't work" in normal play, and
the failure is invisible/confusing because it depends on signer timing the player
can't see.

## 2. Goal — the deliberate-signer model

Stop trying to keep a signer warm during play. Make the signer a **deliberate,
foregrounded action** the player takes only when they want to sync:

- **Sign in is auth-only.** Login proves identity (cookie + cached pubkey) and
  establishes **no** signer. The game loads instantly with no reconnect race.
- **Local Save is signer-free and always works.** Saving writes to this device only.
- **Cloud Sync is explicit.** When the player wants their world in the cloud, they
  open their bunker's timed signing window (on the phone) and hit **Sync**. A
  capable signer is brought up *just for that moment*, the world is uploaded, queued
  feedback flushes in the same window, and the signer may lock again after.
- **Escalation ladder for less friction:**
  - *First sync ever:* a one-time fresh connect (QR / same-device approval) that
    also requests an **auto-sign grant** (the bunker remembers this app).
  - *Every sync after:* no QR — open the bunker window, hit Sync, silent reconnect.
  - *Heartwood (always-on signer):* opt in via a setting; the app keeps a signer
    warm so cloud writes auto-attempt on every save, no Sync button needed.

## 3. Key insight — most of the gating already exists

The engine already stashes only when the world opted in **and** a signer is present:

```rust
// save.rs:2008
pub fn should_stash(world_opted_in: bool, signer_capable: bool) -> bool {
    world_opted_in && signer_capable
}
// save.rs:1251 — the save path
if should_stash(meta.cloud_save, cloud_avail) { /* cloud_save_wasm */ }
```

- `meta.cloud_save` — per-world opt-in, default OFF (`save.rs:1727`).
- `cloud_avail` — comes from `axenstax_cloud_available()` → `AxeCloud.available()`
  → `makeStash() !== null` → **true iff a capable signer is warm right now**
  (`cloud.js:132`, `stashSigner()` returns null unless `capable()`).

So the entire "manual vs always-on" behaviour falls out of **whether a capable
signer is warm**:

| Mode | Signer warm? | `cloud_avail` | Auto-stash on save | Cloud write path |
|------|--------------|---------------|--------------------|------------------|
| **Manual** (default) | No, until Sync | false | never fires | deliberate **Sync** only |
| **Heartwood** (opt-in) | Yes, kept warm | true | fires (today's behaviour) | automatic |

**The single lever is *when we warm a signer*.** The save path, `should_stash`,
`cloud_save` opt-in, and `AxeCloud.available()` are reused unchanged.

## 4. Component changes

### 4.1 `auth.js` — auth-only login, no warm-at-load

- **`runFreshAuth()` (auth.js:275):** request an **auth-only** login from
  signet-login — verify the kind-21236 identity event, establish no bunker signer.
  `_signer` stays `null`. (Existing `retainSigner` becomes a no-op at login because
  there is no signer to retain.)  *⚠ Exact SDK option to be confirmed — see §8.*
- **Returning-user path (auth.js:404–456):** **remove** the unconditional background
  `restoreSession()` bunker reconnect. The `/auth/whoami` cookie identity is enough
  to play. `_signer` stays `null` after load.
- **One exception — Heartwood:** if the always-on flag is set (§4.4), warm the signer
  in the background at load exactly as today (`restoreSession()`), so `cloud_avail`
  becomes true and auto-stash resumes.

### 4.2 `cloud.js` — on-demand signer acquisition + a Sync entry point

New async helper — the heart of the feature:

```
ensureCapableSigner():
  1. capable() ............................ return current signer
  2. else stored bunker session exists? ... await Signet.restoreSession()
                                            retainSigner; re-check capable()
  3. else (first time) .................... await Signet.login({ signer scope,
                                              request auto-sign grant })
                                            retainSigner; re-check capable()
  4. return capable() ? rawSigner() : null
```

- Step 2 is the silent-reconnect-inside-the-open-window path.
- Step 3 is the first-ever-sync fresh connect that asks for the auto-sign grant.
- Returns `null` when the window isn't open / bunker asleep → actionable toast.

New bridge — **the deliberate Sync action**:

```
axenstax_cloud_sync(name, bytes):
  toast('☁ Syncing…', pending)
  signer = await ensureCapableSigner()
  if !signer: toast("Couldn't reach your signer — open your bunker window and Sync again", warn); return ''
  ref = await AxeCloud.save(name, bytes)
  await mailboxFlush()            // same window flushes queued /bug · /idea
  toast('☁ Synced — saved to the cloud', ok); return ref.blobHash
```

- `axenstax_cloud_save` (the auto path, cloud.js:372) is **kept as-is**. In Manual
  mode `cloud_avail` is false so the engine never calls it; in Heartwood mode it
  fires exactly as today. No behavioural branch needed inside it.
- `AxeCloud.available()` / `capable()` / `stashSigner()` unchanged.

### 4.3 Engine (Rust) — Sync trigger + opt-in on sync

- **Sync button** in the in-game pause/escape menu (`menu.rs`). On press, the engine
  serialises the current world (it already does this for `cloud_save_wasm`,
  `wasm_save.rs:215`) and calls a new bridge `axenstax_cloud_sync_wasm(name, blob)`
  → JS `axenstax_cloud_sync`.
- **Sync opts the world in.** On a successful Sync, set `meta.cloud_save = true` for
  that world and persist via the existing `axenstax_set_cloud_save`
  (`wasm_save.rs:56`), so subsequent Heartwood/auto-stash applies to it.
- **Save path unchanged** (`save.rs:1251`). Because `cloud_avail` is false in Manual
  mode, the existing `should_stash` already makes save local-only — no edit needed.

### 4.4 Heartwood / always-on toggle (settings)

- **Source of truth in JS:** `localStorage['axenstax_always_on_' + pubkey]`, default
  OFF (per-persona, per-device). Bridges `axenstax_always_on_get()` /
  `axenstax_always_on_set(on)`.
- **When ON:** auth.js warms the signer at load (§4.1 exception); `cloud_avail` goes
  true; the existing auto-stash fires on every save. No Sync button needed.
- **When OFF (default):** no warm at load; Sync is the only cloud-write path.
- **UI home:** a checkbox in the existing in-game settings panel
  (`menu.rs:draw_settings_panel`, the Spec-39 panel) — "Keep me signed in for cloud
  (always-on signer)". The checkbox is a thin UI over the two bridges; the engine
  stores nothing itself (mode is a JS signer-warming policy, not engine state).
  Pattern mirrors the existing `ui.checkbox` usage (`hud_ui.rs:1245`).

## 5. UX / toast states (top-right, cloud.js `toast`)

| Event | Text | Tone |
|-------|------|------|
| Local save (Manual mode) | *(silent — no cloud toast)* | — |
| Sync started | ☁ Syncing… | pending |
| Sync ok | ☁ Synced — saved to the cloud | ok (green) |
| Sync, no signer reachable | Couldn't reach your signer — open your bunker window and Sync again | warn (amber) |
| Heartwood auto-stash ok / fail | as today ("📦 Stashed…" / "Couldn't stash…") | ok / warn |

Local save success is independent of all of the above and never blocked.

## 6. Feedback payoff (the thread that started this)

`axenstax_cloud_sync` calls `mailboxFlush()` right after the world upload — the same
deliberate, signer-warm window. So `/bug` and `/idea` reports (enqueued locally by
`commands/builtins/feedback.rs` → `wasm_feedback` → `window.__axenstax_mailbox_enqueue`)
now have a **reliable** flush moment with a guaranteed-capable signer, instead of
depending on a signer happening to be warm during play. This design **is** the fix
for "feedback doesn't send."

## 7. Data / persistence

- **Bunker session:** stored by signet-login on first connect; reused by
  `restoreSession()`. We change only *when* we restore (at Sync / at load-if-Heartwood,
  never unconditionally at load). No new storage from us.
- **Heartwood flag:** one localStorage boolean per persona (§4.4).
- **Per-world opt-in:** existing `WorldMeta.cloud_save`, now also set by Sync.
- **No save-format change.** No protocol change.

## 8. Dependencies & risks — VERIFY DURING PLANNING

These are owner/upstream (Signet) concerns; AxeNStax only consumes them. Per the
Signet boundary, none of this is AxeNStax-specific Signet work.

1. **🚩 signet-login API surface (top risk).** This design assumes the vendored SDK
   exposes (a) an **auth-only login mode**, and (b) a way to bring a capable bunker
   signer up at Sync — `restoreSession()` inside an open window, plus a first-time
   connect that requests an **auto-sign grant**. The vendored `REGENERATE.md` only
   documents `login / restoreSession / handleRedirectCallback / handleCallback /
   logout` — no explicit auth-only flag or signing-window/grant API is visible.
   **Planning must confirm against the live SDK and may require a vendored bundle
   bump and/or alignment with the owner's signet-app #184 (timed bunker window) +
   auto-sign work.** Memory note: extra-persona auth-only landed upstream (#182).
2. **Re-acquiring signer scope post-auth.** Whether `Signet.login()` can be called a
   second time (at first Sync) to add signer capability for the already-authed
   identity *without* a full re-auth, or whether a dedicated connect call is needed.
3. **Auto-sign grant** (bunker remembers the app, no per-event approval) is
   bunker/Signet-app side; we assume "remembered" after the first grant.
4. **Heartwood** (always-on signer infrastructure) is future/owner-owned. We ship
   only the toggle and treat the signer as always-warm when it's on.

If (1) shows the SDK can't yet do auth-only or sync-time warming, the fallback is the
"pair-but-stay-cold" variant (login pairs the bunker but never warms it during play;
Sync `restoreSession()`s) — strictly worse UX at sign-in, kept only as a contingency.

## 9. Testing

- **JS (cloud.js / auth.js):** `ensureCapableSigner` state machine (capable → return;
  paired → restore; unpaired → fresh connect; window-closed → null); Sync flushes
  feedback; Manual mode performs no warm-at-load; Heartwood mode warms at load. Mock
  `window.Signet` + signer + relay (mailbox already has an in-memory harness).
- **Engine (Rust):** Sync button serialises current world + calls the bridge + sets
  `cloud_save = true`; settings checkbox round-trips the always-on bridges; save path
  stays local-only when `cloud_avail` is false (existing `should_stash` tests cover
  the gate — keep them green). Runs under `check.sh`.

## 10. Out of scope (v1)

- Heartwood always-on signer infrastructure (owner/upstream).
- The Signet-app timed-window UI + auto-sign grant mechanics (owner/upstream; live).
- **Lobby Sync button** — v1 ships in-game Sync only; the lobby already has a
  feedback-only "Sync to send" (`lobby.js:358`). A world-list Sync is a fast-follow.
- Multiplayer-identity signing (separate, Spec 1 Phase 4 / engine signing bridge).

## 11. Spec / doc touchpoints to update on implementation

- `docs/foundations/2026-06-07-lobby-mailbox-feedback.md` — flush now rides the
  deliberate Sync (and Heartwood auto-stash), not an incidental save.
- The cloud-save / Stash notes wherever the auto-stash-on-save coupling is described.
- `CLAUDE.md` — Signet "Key Patterns" section: sign-in is auth-only; signer is warmed
  on demand at Sync (or kept warm under Heartwood).

## 12. Brainstorm decisions captured

- Approach **A** (pair on first Sync; login is *truly* auth-only). Chosen over
  pair-at-login-stay-cold (B) and no-modes-just-silence (C).
- Save/Sync **fully decoupled** (a): Save = local-only; cloud only via Sync.
- Heartwood is a **settings toggle**; ON → auto-attempt on every action, no Sync.
- Timed bunker window is **live now** per owner → design calls into it directly.
