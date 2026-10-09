# Agent instructions

This repo's instructions for coding agents live in **`CLAUDE.md`** at the repo root. Read it first and treat it as authoritative. It holds:
- the build and verification commands (`./check.sh` is the single regression gate);
- the **Regulatory Red Lines** (load-bearing: check every networking, discovery, identity, hosting, data or public-copy change against them);
- the rule not to build anything unless explicitly asked;
- spec maintenance (the specs in `docs/spec/` are the source of truth and change with behaviour);
- the code-quality rules and the known technical debt.

## Working rules that apply to every agent

- UK English in code comments, docs and UI text.
- Show a Nostr identity as an `npub`, never as hex.
- Stage with `git add <paths>`, never `git add -A` or `git add .`: other sessions may share this checkout.
- Never force-push and never rewrite shared history. Never push a branch other than `main` to `origin`.
- A push to `main` deploys the websites. Ask the maintainer before pushing.
- Run one build at a time, in the foreground. Builds are heavy, and parallel cargo jobs can take the machine down.

## Maintainer-machine coordination (only if present)

`.claude/` is gitignored, so these files exist only on the maintainer's machine. If they are there, use them:
- `.claude/epic/STATUS.md`: the live state of the current multi-lane work. Resume from its "RESUME HERE" block and the lines after it.
- `.claude/epic/briefs/`: one brief per lane. Each has its decisions, scope, tests, acceptance steps and report shape. Follow a brief as written.
- `.claude/epic/handoff/`: evidence files, verify reports, and each lane's progress file (resume a paused lane from its brief plus its progress file).
- `.claude/epic/MORNING-REPORT.md`: items waiting on the maintainer.
- `.claude/epic/build.sh`: the build wrapper. Run every `cargo`, `trunk` and `./check.sh` invocation through it. It serialises builds and cleans stale artefacts when the tree changes.
- `.claude/worktrees/`: one git worktree per lane. Work only in the worktree a brief names.

Briefs may name model tiers or commit trailers specific to the tool that wrote them. Keep the brief's decisions, tests and acceptance steps; adapt the tool-specific parts.
