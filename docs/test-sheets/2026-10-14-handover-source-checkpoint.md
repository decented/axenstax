# Handover source checkpoint — 14 October 2026

This is a private, resumable source checkpoint on top of 91d489f7, with final
feature acceptance pending. Engine 0.2.27, private wire 90 (Handover 72,
PersonalSync 73), canonical participant save v3, WorldSave 53 fields and save
layout revision 0. Mirror remains the default; Strict remains partial/parked.

Automated checkpoint checks passed:

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

The original-invitation tests reconstruct verified epoch 0→1→2→3 routing; they
do not yet execute three complete live transfers. Retained current-state
coverage reopens the canonical helper, and does not claim the full native
selected-WorldSave path. Fixtures use loopback relays and direct test transport.

Still required before the concrete two-machine playtest:

- [ ] Full actual original-invitation 0→1, hand-back 2, later retirement 2→3.
- [ ] Strict frozen source metadata read; missing/torn metadata refusal without
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
