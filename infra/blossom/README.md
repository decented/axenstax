# infra/blossom/ — RETIRED

This directory held a Blossom blob store for the old web cloud-save "bridge
tier", reached by the game site over loopback. That tier is gone: the web build
is an anonymous offline taster that talks only to its own origin and has no
cloud save (`tools/sites/game/app.py` reads no Blossom setting), and nothing in
the deploy or CI files starts this store. Native Stash is bring-your-own-key and
non-custodial.

The compose file, config and scripts are kept only as a record of the retired
design. Nothing should be built or run from here. Background:
`docs/foundations/2026-05-26-cloud-save-blossom.md`.
