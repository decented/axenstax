# Alpha onboarding for Charter pairing — UX surfaces + docs

**Status**: **SUPERSEDED 2026-05-09 by Charter spec rev. 7** (mechanism A — static-data evaluation). The "consumer-side pairing-intake UX" this spec scopes does not exist in the rev. 7 contract — pairing in mechanism A is implicit in the existing URL-auth handshake (no `bunker://` paste-box, no scan, no Pair button). The walkthrough doc + FAQ this spec described would actively mislead alpha testers under rev. 7. Phase 1 + Phase 2 deliveries from this spec landed 2026-05-08 then were reverted 2026-05-09 as part of the rev. 7 cleanup; the reverted commits live on branch `preserved/charter-rev5-groundwork` for reference.
**Date**: 2026-05-08 (status updated 2026-05-09).
**Memory rules in scope**: npub only display (any handle/identity surface uses npub).
**Replacement**: a rev. 7-shaped onboarding spec gets drafted alongside the rev. 7 integration work. Will be much smaller — possibly a single FAQ entry plus a one-line note in the existing welcome card.

---

## TL;DR

The `bunker://` pairing model is **per-installation**: a parent who pairs AxeNStax for Tom on the iPad has not paired AxeNStax for Tom on the desktop. Without explicit onboarding, alpha testers will hit "I thought I already paired this" friction and assume the integration is broken.

Three small artefacts close the loop:

1. **A pairing-intake UX surface in `lobby.html`** — paste-box (primary), QR-scan (polish), with a clear "Pair AxeNStax with Signet for parental controls (optional)" framing.
2. **An alpha onboarding doc** at `docs/alpha-onboarding/charter-pairing.md` covering the mental model + per-device flow + first-time-user experience.
3. **A FAQ entry** for the most likely confusion: "I paired AxeNStax for my kid on the iPad — why is the desktop asking again?"

This spec is small but real — skipping it means alpha testers DM Staxolottle instead of self-serving.

---

## Context pointers

- Pairing UX flow on the signet-app side: `~/Documents/<workspace>/forgesworn/signet-plans/docs/plans/2026-05-08-charter-schedule-clause-spec.md` §Q5 Phase 1 "Pairing UX flow" rev. 3 answer.
- Storage backing the pairing intake: `2026-05-08-charter-pairing-storage.md` — `axenstax_pair_charter(uri, subject)` is what the intake UX calls.
- Existing onboarding-style docs in the project: `docs/integrations/signet/2026-04-20-accept-hint-consumer-wire-up.md` (consumer-side wire-up doc; similar tone-target).
- Existing alpha admin: `docs/foundations/...request-access...` and `tools/sites/game/templates/admin_requests.html` — the alpha experience already has an admin surface for whitelisting.
- Memory alpha access admin — the admin page is `/admin/requests?token=$ADMIN_TOKEN`.

---

## Surface 1 — Pairing-intake UX in `lobby.html`

**Where**: in the lobby (`tools/sites/game/templates/lobby.html`), behind a small disclosure: *"Charter parental controls (optional)"*. Default-collapsed so non-Charter testers don't see noise.

**What it shows**:

- A short explainer (2 lines): *"If your guardian has paired this device with their signet-app, paste or scan the bunker:// link below."*
- A `<textarea>` for paste — primary intake.
- A "Scan QR" button (polish — opens device camera if `navigator.mediaDevices.getUserMedia` is available; falls back gracefully).
- A "Pair" button that calls `axenstax_pair_charter(uri, current_signed_in_pubkey)`.
- Success → green banner: *"Paired. Charter checks will run silently from now on."* + render the pairing as a row in a small "Paired devices" list (npub-shortened: `npub1abc...xyz`).
- Failure → red banner with the specific reason (`invalid URI`, `pairing failed: <bunker error>`).

**What it does NOT show**:

- The full bunker URI after pairing (treat as a credential — only show length + a 6-char prefix).
- Any clause-level data (clause editing is the parent's job inside signet-app, not AxeNStax).
- Re-pair friction: if a pairing already exists for this subject, the "Pair" button label changes to "Re-pair" and shows a confirmation prompt before overwriting. Don't make accidental re-pairs easy.

**Where in the flow**: visible after sign-in (kid pubkey is known). Before sign-in, the pairing-intake is hidden — there's no `subject_pubkey` to bind the pairing to until the kid is signed in.

**State machine**:

| State | Trigger | UX |
|---|---|---|
| Hidden | Not signed in | Disclosure not rendered |
| Collapsed | Signed in, no pairing | Disclosure summary: "Charter parental controls (optional)" |
| Collapsed | Signed in, has pairing | Disclosure summary: "Charter paired (last used <timestamp>)" |
| Expanded | User clicks disclosure | Paste-box + scan button + "Pair" / "Re-pair" / "Unpair" |

---

## Surface 2 — Alpha onboarding doc

**Path**: `docs/alpha-onboarding/charter-pairing.md`. Create the `alpha-onboarding/` directory; this is the seed doc.

**Audience**: alpha-tester parents. Tone: friendly, concrete, links to signet-app where relevant.

**Outline** (~250 words target):

1. **What Charter does** (2 lines). Schedule clauses + bunker-side enforcement. Link to the public Charter explainer when one exists.
2. **The mental model**: pairing = "I'm telling Signet that Tom's allowed to play AxeNStax under my Charter clauses." Once paired, AxeNStax silently asks Signet at every session start whether Tom can play *right now*.
3. **How to pair** (step-by-step):
   - Open signet-app on your phone.
   - Go to Tom's dependant settings → "Pair an app for Tom".
   - Generate a `bunker://` URI. Either (a) copy it and paste into AxeNStax on the same device, or (b) display the QR and scan it from AxeNStax on Tom's tablet/desktop.
   - In AxeNStax, paste the URI into the Charter pairing box and click Pair.
   - Done. Tom can play; AxeNStax checks Charter silently each session start.
4. **Per-device note**: each device Tom plays on needs its own pairing. iPad → pair on iPad. Desktop → pair on desktop. This is by design — it's how the bunker keeps the pairing private to the device.
5. **What if Tom hasn't been paired**: he can still play — Charter is opt-in. AxeNStax notes "no pairing" silently. If a parent wants enforcement, they pair.
6. **What if pairing fails**: usually means signet-app is offline or a typo in the URI. Try again, or copy the URI fresh from signet-app.
7. **Removing a pairing**: signet-app side — go to Tom's paired apps → revoke. Or AxeNStax side — disclosure → "Unpair" button (clears the pairing on this device only; signet-app still has it).

---

## Surface 3 — FAQ entry

**Path**: `docs/alpha-onboarding/faq.md` — create if it doesn't exist; otherwise append.

**Entries**:

> **I paired AxeNStax for my kid on the iPad — why is the desktop asking again?**
> The bunker pairing is per-device by design. Each browser/device that Tom plays on needs its own pairing, so the encrypted connection stays scoped to that device. Pair AxeNStax on the desktop the same way you paired the iPad: open signet-app → Tom's dependant settings → "Pair an app for Tom" → paste the URI into AxeNStax on the desktop.

> **Will my kid know when AxeNStax is checking Charter?**
> No. After the one-time pairing, every session-start check happens silently — the kid doesn't see a consent dialog or a "checking Charter…" spinner. The only time the kid sees Charter is on a deny ("Time's up, see you tomorrow at 4pm").

> **My bunker is offline. Can my kid still play?**
> If you'd previously paired AxeNStax with bunker for Tom: no. Charter fail-closes at session start when the bunker is unreachable, so Tom sees "Can't reach your Charter — try again in a moment." This is a deliberate safety choice. Once the bunker reconnects, Tom can play.
> If you've never paired AxeNStax with the bunker: yes. Tom plays without Charter checks. Charter is opt-in.

---

## Phased plan

### Phase 1 — Onboarding doc + FAQ (~1 hour, doc-only)

- Create `docs/alpha-onboarding/` directory.
- Write `charter-pairing.md` per the outline above.
- Create `faq.md` with the three Q/A entries.
- Add a link from `docs/foundations/README.md` → `alpha-onboarding/charter-pairing.md` for visibility.

### Phase 2 — `lobby.html` pairing-intake UX (~4 hours)

- Add the disclosure surface to `lobby.html`.
- Wire `Pair` / `Re-pair` / `Unpair` buttons to the storage-spec API (`window.axenstax_pair_charter` etc.).
- Render the pairing list with npub-shortened display (per npub only display).
- Smoke against a mock `bunker://` URI.

### Phase 3 — QR-scan polish (~2 hours, optional)

- Add the camera-scan flow using `navigator.mediaDevices.getUserMedia` + an existing QR-decoding lib.
- Hide the button gracefully when the API is unavailable (older browsers, no permission).
- Ship without QR-scan if Phase 2 is enough for alpha — paste-box covers all the same scenarios with a few extra clicks.

### Phase 4 — Verification

- Manual: alpha-tester walk-through against a dev signet-app instance. Pair → play → see silent Charter check → unpair → still play (no-pairing path).
- Doc walkthrough: ask a non-engineer to read `charter-pairing.md` and pair without further help. If they can, the doc's done.

---

## Acceptance

- `docs/alpha-onboarding/charter-pairing.md` exists, ~250 words, walks a parent through pairing without further help.
- `docs/alpha-onboarding/faq.md` answers the three most-likely confusions.
- `lobby.html` exposes the pairing-intake disclosure with paste + Pair/Re-pair/Unpair.
- Pairing UX uses npub for any displayed identity, never raw hex.
- Alpha testers can self-serve "I thought I already paired this" without DMing Staxolottle.

---

## What this spec does *not* cover

- Engine-side deny-screen UX — that's the Charter Phase 1 implementation work, in `menu.rs` (Rust, not site JS).
- The actual storage layer — that's `2026-05-08-charter-pairing-storage.md`.
- Loading `nostr-tools/nip46` — that's `2026-05-08-site-build-pipeline-for-npm.md`.
- Charter contract changes — those go to Forgesworn against the schedule-clause spec.

---

## Memory rules check

- npub only display — pairing list shows npub, internal storage stays hex. ✓
- alpha access admin — alpha admin already exists; this spec is the alpha *onboarding* layer. Different surface. ✓
- pretest check — Phase 1 onboarding doc is testable solo (read it, can a non-engineer follow?). Phase 2/3 testable against a dev signet-app. ✓
