# platform/ — RETIRED

The directories here (`matchmaking/`, `orchestration/`, `payments/`) are empty
placeholders from an early plan for an AxeNStax-operated platform: a session
directory and matchmaker, an Agones game-server fleet, and hosted payment
plumbing. **That plan is retired.** An AxeNStax-run public directory, game
servers or payment service would make AxeNStax the operator of a regulated
service rather than a software vendor. Worlds are self-hosted; discovery is
LAN-local, opt-in self-published Nostr announce, or direct address. Nothing
should be built here. See `docs/spec/07-platform-services.md` and the red lines
in `CONTRIBUTING.md`.
