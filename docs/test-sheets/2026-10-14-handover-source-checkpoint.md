# Handover source checkpoint — 14 October 2026

This is a private, resumable source checkpoint after normal integration of
visual e59b5e36 on 23752f9f, with final feature acceptance pending. Engine 0.2.27, private wire 90 (Handover 72,
PersonalSync 73), canonical participant save v3, WorldSave 53 fields and save
layout revision 0. Mirror remains the default; Strict remains partial/parked.

Earlier checkpoint checks passed (87 distinct focused controls):

- Native Clippy with warnings as errors.
- Three original-invitation cold lookup/chain controls and one returned-runtime
  stale proof/contact/copy/abort control.
- Actual activated-import host-start replacement and persistent pending/terminal
  abort refusal across reopen.
- Three retained-world controls: source-own current canonical state and signed
  provenance replant refusal, inert GameServer/HostedServer opens, and duplicate
  local source owner refusal alongside former-recipient admission.
- Sixteen personal codec/bootstrap/current-snapshot/role/delivery tests, one
  transfer-seed bound test, two portable codec tests, four canonical save tests,
  forty-eight protocol tests and eight save-format tests.
- Four deliberate guard removals each failed at their intended runtime
  assertion; all source bytes were restored exactly and the controls passed
  again. Compiler/fixture setup diagnoses were recorded separately.

Additional checkpoint checks passed:

- Strict bounded native metadata parsing accepts healthy pretty JSON and known
  historical fields, while archive metadata remains byte-canonical.
- Actual frozen GameState capture refuses both missing and torn metadata,
  preserves source bytes, tells the recipient and resumes the original play.
- Three complete original-invitation controller-issued transfers: root 0→1,
  hand-back 2 and returned-source retirement 2→3. Each hop uses the production
  preparation resolver, common host-start folder-proof check before server
  construction, encrypted Offer/Answer verification and before-contact authority
  gate. Repeated Host/copy preserves the original invitation. Native promoted
  identity-selected state survives changes, save/reopen and the next transfer.
- Deliberately restoring the old metadata default/recovery fallback fails at the
  actual capture-refusal assertion. All eleven candidate source hashes restore
  exactly, Clippy passes again, and the refusal control returns green. The
  restored executable SHA matches the earlier passing three-hop executable.

These controls use a loopback stored relay, an in-memory online setup relay and
an injected direct game channel after the real Answer is verified. They do not
claim a two-machine router or public-relay playtest. The retained current-state
helper and released-proof assertions remain separate from complete native
retained-copy, fault and portability acceptance.

Still required before the concrete two-machine playtest:

- [x] Checkpoint actual original-invitation 0→1, hand-back 2 and retirement 2→3.
- [x] Strict frozen source metadata read; missing/torn metadata refusal without
      display/default recovery or changed source bytes.
- [ ] Actual promoted/retained native load, matching save/autosave/discard,
      dedicated role continuation, delayed authorisation and Plan restoration.
- [ ] Retained promotion/directory-sync faults, restart retries, non-overwriting
      collision handling and complete-folder portability.
- [ ] Normal visual/packs integration, independent review and complete immutable
      native/GPU/WASM/check.sh regression.
- [ ] Concrete native build and two-machine source→recipient→hand-back sheet.

Do not use this checkpoint as an accepted-feature build or infer support for
active quests or complete personal timer continuation. The final player sheet
will identify the exact tested integrated commit and native binary.
