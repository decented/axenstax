# Dedicated-server image publish — design spec

**Date:** 2026-06-18 · **Status:** DESIGN (not built) · **Author:** autonomous build session
**Depends on:** `tools/dedicated-server/` (built), `docs/superpowers/specs/2026-06-16-dedicated-docker-server-design.md`
**Sibling:** `docs/operators/dedicated-server.md` (the live-site guide this unblocks)

---

## 1. The problem

Today, "self-host an Axe'n'Stax server" means: **install the Rust toolchain +
`trunk`, clone the (private) source tree, host-compile the binary + WASM bundle
(`build.sh`), then `docker compose build && up`.** That is a *developer* workflow.
An alpha tester will not do it. The live-site guide can describe it, but the
experience is gated behind being a Rust developer with repo access.

**Goal:** a published, `docker pull`-able image so a self-hoster runs **two
commands and nothing else** — no toolchain, no source checkout:

```bash
curl -O https://.../docker-compose.yml      # or copy the 30-line file
docker compose up -d
```

This turns the `tools/dedicated-server/README.md` "fully self-contained, cross-arch
image … is the next step" note into reality, and is blocker **#2** of the three
that currently stop alphas self-hosting (the other two — repo visibility and
publishing the guide — are addressed separately; the guide is now live).

## 2. Current state (what exists)

- `tools/dedicated-server/Dockerfile` — packages a **host-built** binary + web
  bundle (`COPY stage/…`); does *not* compile from source. Arch-aware only for the
  Caddy static binary, not the engine.
- `tools/dedicated-server/build.sh` — host-compiles (`cargo build --release`,
  `trunk build`), stages artifacts, `docker compose build`.
- `tools/dedicated-server/docker-compose.yml` — `build:` context + local image tags
  (`axenstax-dedicated-server`, `axenstax-operator-console`). No registry.
- `.github/workflows/native-packages.yml` — the existing pattern to mirror:
  `workflow_dispatch` with a `linux_only` boolean **defaulting to `true`** to skip
  metered macOS/Windows runners; also a narrow `push` trigger on a dedicated branch.
- **Constraint:** the repo is **private → Actions minutes are metered.** A full
  Rust + WASM compile is heavy; doing it ×2 architectures multiplies the cost.

## 3. Design

### 3.1 Where the compile happens — **multi-stage build-in-Docker** (recommended)

Add a new **`Dockerfile.publish`** (multi-stage) used *only* by the publish
workflow. Leave the existing host-build `Dockerfile` + `build.sh` untouched as the
fast local-dev path.

```
# stage 1: builder (rust:1.8x-bookworm) — cargo build --release --bin axenstax-engine
#                                          + trunk build --release index.dedicated.html
# stage 2: runtime (debian:bookworm-slim) — COPY --from=builder the binary + dist/
#                                          + Caddy (as today) + entrypoint
```

Why this over "CI runs build.sh then COPY":
- **Self-contained + reproducible** from a git ref — no host state, no staged dir.
- **Multi-arch falls out** of `docker buildx --platform linux/amd64,linux/arm64`
  (the builder stage compiles for each target) — this is what finally removes the
  arm64-NAS host-build requirement.
- It's the end state the README already promised; the host-build path stays as the
  dev shortcut (fast incremental local iteration), so we keep both, each labelled.

**Trade-off:** building Rust + WASM inside Docker is slower than the host build
(no warm cargo cache across runs unless we add registry layer caching). Mitigated
by (a) GHA cache / `--cache-from` on the builder stage, and (b) a release-cadence
trigger, not per-push (see 3.4).

### 3.2 Registry — **GHCR (`ghcr.io`)** (recommended)

- Native to GitHub Actions; auth via the built-in `GITHUB_TOKEN` (no extra
  account/secret), `permissions: packages: write`.
- **A GHCR package can be made public independently of the repo's visibility.** So
  we can ship a *pullable image* while the source repo is still private — this
  directly unblocks alphas without opening the source. (Owner decision: 4-a.)
- Image refs: `ghcr.io/<org>/axenstax-dedicated-server` and (see 3.6)
  `ghcr.io/<org>/axenstax-operator-console`.

Alternative considered: **Docker Hub** — more familiar `docker pull` namespace, but
needs a separate account + secret and rate-limits anonymous pulls harder. GHCR wins
on zero-extra-setup and repo coupling. (Decision 4-b.)

> **One-time prerequisite — link the package to the repo (gotcha, hit 2026-06-22).**
> `permissions: packages: write` grants the Actions `GITHUB_TOKEN` the *scope* to
> write packages, but GHCR also enforces *per-package* access: the token can only
> push to a package **linked to its repository**. A package first created by a
> **local `docker push`** (with a personal PAT that has `write:packages`) is owned
> by the user with `repository = null` and is **not** writable by CI — the workflow
> builds for ~16 min and then dies at the push step with
> `denied: permission_denied: write_package`. This bit us on the first
> `workflow_dispatch` run: both `axenstax-server` and `axenstax-operator-console`
> had been pushed locally on 2026-06-18 and showed `repository: null`.
>
> **Fix (once per package, GitHub UI — no REST endpoint exists for user-owned
> packages):** `https://github.com/users/<owner>/packages/container/<pkg>/settings`
> → "Manage Actions access" → Add Repository → `decented/axenstax` → role **Write**.
> Alternatively, delete the unlinked package so the first *Actions* push recreates
> it auto-linked (costs a brief `:latest` gap for any live puller).
>
> **Don't verify via `.repository`** — granting access through "Manage Actions
> access" does **not** set the package's `.repository` field (that's only set by
> "Connect repository" or a first push *from* Actions), so `gh api
> /users/<owner>/packages/container/<pkg> --jq .repository` can read `null` even
> when CI push is correctly granted. The authoritative check is whether GHCR's
> token endpoint grants `push` to the repo's `GITHUB_TOKEN`. The workflow's
> **Pre-flight step** does exactly that (asks for a `pull,push`-scoped token and
> inspects the granted actions — pushes nothing) and fails before the long build,
> printing the exact fix URL.

### 3.3 Tags

`:latest` (moving), `:<short-sha>` (immutable, traceable), and `:<tag>` when the
build is triggered by a `git tag` (e.g. `server-v0.1.0`). The published compose
pins `:latest` for alphas; cautious operators pin a sha/semver.

### 3.4 Trigger & cadence — **dispatch + tag, never per-push** (cost control)

Metered minutes + a heavy compile ⇒ **do not build on every push to main.** Mirror
`native-packages.yml`:

- **`workflow_dispatch`** with inputs:
  - `multiarch` (boolean, **default `false`**) — `false` builds `linux/amd64` only
    (one cheap-ish compile); `true` adds `linux/arm64` (emulated, slow — see below).
  - `push` (boolean, **default `false`**) — dry-run by default (build + smoke-test,
    no registry push); set `true` to publish. *(Same "build artifacts without
    publishing" safety as the native pipeline.)*
- **`push` on `tag: ['server-v*']`** — a tagged release builds multiarch + pushes.

This keeps routine CI spend at zero and makes every publish an explicit act.

**arm64 cost flag:** `linux/arm64` under QEMU emulation compiles Rust ~5–10× slower
than native — a multiarch publish can run long and burn minutes. Options, cheapest
first: (i) amd64-only for alpha, document "arm64 NAS: build locally for now";
(ii) emulated arm64 on release cadence only; (iii) a native arm64 runner if/when
available. Recommend **(i) for the first cut**, revisit when an arm64 community
actually needs it. (Decision 4-c.)

### 3.5 Compose: pull-and-run by default

Ship the operator-facing compose pointing at the registry image:

```yaml
services:
  axenstax-server:
    image: ghcr.io/<org>/axenstax-dedicated-server:latest
    # no build: — pulled, not built
    ...
  axenstax-console:
    image: ghcr.io/<org>/axenstax-operator-console:latest
    ...
```

Keep the current `build:`-based compose for source builds as
`docker-compose.build.yml` (or behind `build.sh`). The default file an alpha copies
is the pull one. `docker compose pull && up -d` then needs **only Docker.**

### 3.6 Scope: publish **both** images

The `/admin` Operator Console is a second image (`tools/sites/console`). For a
genuinely pull-and-run stack it must be published the same way (it builds fast —
no Rust — so cost is negligible). The workflow builds and pushes **both** the
server and the console image in one run. (Decision 4-d — include console: yes.)

### 3.7 Verify before publish

The workflow **smoke-tests the built image before pushing**: `docker run` it, wait
for health, `curl -k https://localhost:8443/` (web root) and the console
`/healthz`, assert 200s. A red smoke test fails the run — we never push a broken
image. (No engine source changes; the engine is already gated by `check.sh`.)

### 3.8 Security / provenance

- **No secrets in the image.** Operator identity is paired at *runtime* onto the
  worlds volume (`--pair-server`); nothing sensitive is baked in. The image is safe
  to be public.
- `GITHUB_TOKEN` with `packages: write`, nothing broader.
- **Future (not v1):** `cosign` image signing + GitHub build-provenance attestation,
  so operators can verify the image was built by our CI from a known ref. Flagged,
  deferred.

## 4. Decisions for owner review

| # | Decision | Recommendation |
|---|----------|----------------|
| 4-a | Make the GHCR **package public** while the repo is still private? | **Yes** — it's the whole point (pullable image without opening source); image carries no secrets. |
| 4-b | Registry | **GHCR** (zero extra setup, public-package-independent-of-repo). |
| 4-c | Multi-arch from day one? | **amd64-first**; document arm64-NAS local build; add emulated arm64 on release cadence later. |
| 4-d | Publish the **console** image too? | **Yes** — required for pull-and-run `/admin`; cheap to build. |
| 4-e | Build cadence | **`workflow_dispatch` (dry-run default) + `server-v*` tag**; never per-push (metered minutes). |

## 5. What this does *not* cover

- Making the **source repo** public (blocker #3) — separate owner decision
  ("opening soon"). This spec deliberately ships a usable image *without* it.
- The Spec 07 Agones **fleet** — this is the single self-hostable server only.
- An auto-update mechanism for running servers (operators `docker compose pull`
  manually for now).

## 6. Implementation outline (for the eventual plan — not built here)

1. `tools/dedicated-server/Dockerfile.publish` — multi-stage (rust builder → slim
   runtime), `ARG TARGETARCH`/`TARGETPLATFORM` aware.
2. `tools/dedicated-server/docker-compose.yml` → registry `image:` refs (pull);
   move the `build:` variant to `docker-compose.build.yml`.
3. `.github/workflows/publish-server-image.yml` — `workflow_dispatch`
   (`multiarch`/`push` inputs, dry-run default) + `server-v*` tag; `buildx`,
   GHA layer cache, smoke-test gate, GHCR push of **both** images.
4. Update `docs/operators/dedicated-server.md` path **A** from "coming soon" to the
   live `docker compose up -d` once the package is public.
5. (Future) cosign signing + provenance attestation.

**Cost note for the build session:** the *first* multiarch publish run is the
expensive one (cold cache, emulated arm64). Run it **deliberately** (dispatch,
`push:true`), confirm the metered-minute spend is acceptable, and keep routine CI
at the amd64 dry-run default. Per project rule, **confirm before triggering the
first metered publish run.**
