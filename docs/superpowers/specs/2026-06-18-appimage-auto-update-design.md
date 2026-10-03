# AppImage auto-update — design spec

**Date:** 2026-06-18 · **Status:** IMPLEMENTED 2026-06-24 — CI re-stamp + publish + docs
route all wired; first live publish is **v0.2.1** (parity with the deployed WASM/PWA build).
**Context:** owner dropped `.deb`, kept **AppImage as the one Linux artifact**, and wants it
to auto-update (the standard AppImage delta-update experience). Homepage already reflects
AppImage-only (`feat(marketing): two-door hero`).

---

## 1. Goal

The downloaded `.AppImage` updates itself to new releases via the **standard AppImage
zsync delta mechanism** — download only the changed chunks, replace in place. No app
store, no reinstall.

## 2. How it works (the mechanism)

Two ingredients, both produced at build time:

1. **Embedded update information** — a string baked into the AppImage's `.upd_info` ELF
   section telling an updater where new versions live. Our transport (we publish to our
   own `/download`, **not** GitHub Releases):
   ```
   zsync|https://docs.axenstax.org/download/axenstax-engine-latest-x86_64.AppImage.zsync
   ```
2. **A companion `.zsync` file** published next to the AppImage. The updater fetches it,
   diffs against the local file, and pulls only the changed blocks.

The **trigger** (what actually runs the update):
- **v1 (this spec):** the user runs **AppImageUpdate** or has **AppImageLauncher** installed
  (it offers updates automatically). We document this one line on `/download`.
- **v2 (deferred):** the engine self-checks on launch and shows "update available." That's
  an *engine* change (bucket-2 boundary — packaging must not touch engine source), so it's
  a separate spec.

## 3. The stable-URL rule (the crux)

The embedded URL is **frozen into every build**, so it must never change across versions.
Therefore publish, for each release, **two names**:
- the **versioned** file — `axenstax-engine_<ver>_x86_64.AppImage` (what `/download` lists), and
- a **stable `latest` copy** — `axenstax-engine-latest-x86_64.AppImage` **+** its
  `…-latest-x86_64.AppImage.zsync` — which is what the embedded `zsync|…` URL points at.

The `.zsync`'s internal header also stores the URL of the actual AppImage to fetch; point
that at the `latest` AppImage too. Net: the embedded info is identical in every build, and
"latest" always resolves to the newest release.

## 4. The build recipe (cargo-packager lacks update-info → post-process)

cargo-packager 0.11.8 builds the AppImage but exposes **no** AppImage update-information
field (its `signer` is for the unrelated Tauri-style updater). So after cargo-packager
produces the `.AppImage`, re-stamp it with `appimagetool`, which writes `.upd_info` **and**
emits the `.zsync`:

```bash
# given cargo-packager's output: <staging>/axenstax-engine_<ver>_x86_64.AppImage
UPD='zsync|https://docs.axenstax.org/download/axenstax-engine-latest-x86_64.AppImage.zsync'

./axenstax-engine_<ver>_x86_64.AppImage --appimage-extract          # → ./squashfs-root (no FUSE needed)
ARCH=x86_64 appimagetool -u "$UPD" squashfs-root \
    axenstax-engine-latest-x86_64.AppImage                          # stamps .upd_info + emits .zsync
# also keep a versioned copy for the /download listing:
cp axenstax-engine-latest-x86_64.AppImage axenstax-engine_<ver>_x86_64.AppImage
```

`appimagetool` is the same tool cargo-packager already fetches to build AppImages, so the
toolchain is present in CI/local (the v0.1.0 AppImage built + launched locally — README).
Verify the stamp locally with `readelf -p .upd_info <file>.AppImage` (must echo the URL).

## 5. Pipeline changes

| File | Change | Status |
|------|--------|--------|
| `tools/packaging/build-local.sh` | after the cargo-packager appimage build, run the §4 re-stamp; output the `-latest-` AppImage + `.zsync` alongside the versioned one | ✅ done |
| `.github/workflows/native-packages.yml` | same re-stamp in the Linux job; upload the `-latest-` AppImage **and** the `.zsync` as artifacts | ✅ done 2026-06-24 |
| `.github/workflows/publish-installers.yml` | rsync the `.zsync` and the `-latest-` AppImage into `INSTALLERS_DIR` (`/opt/axenstax/build/installers`) alongside the versioned one | ✅ done 2026-06-24 (`*.zsync` added to the flatten; `-latest-` rides as `*.appimage`) |
| `tools/sites/docs/app.py` | **gap found 2026-06-24:** the embedded URL is `/download/<file>` but only `/download/installer/{name}` + `/download/axenstax-engine` existed — the bare `.zsync` URL 404'd. Added a tightly-scoped `/download/{name}` route serving only `_AUTOUPDATE_NAMES` (the `-latest-` AppImage + its `.zsync`); `_discover_installers` now hides `-latest-` so the page lists only the versioned download. | ✅ done 2026-06-24 |

## 6. Boundary (what's owner-gated)

- **Buildable + locally verifiable now:** the §4 re-stamp recipe + `build-local.sh` change —
  build an AppImage, confirm `.upd_info` carries the URL and the `.zsync` is emitted.
- **Owner-gated / metered:** the **live publish** — `native-packages.yml` is dry-run by
  design ("outward publish is owner-gated"), and the real delta-update can only be proven
  once the `-latest-` AppImage + `.zsync` are live on `/download`. So: I prep + locally
  verify; **you** trigger the publish, then we confirm a real `AppImageUpdate` delta.

## 7. Out of scope / deferred

- **In-app update check** (v2) — engine self-checks on launch. Engine-source change → its
  own spec, after this lands.
- **`.deb`** — dropped per owner. AppImage is the sole Linux artifact.
- **Windows/macOS auto-update** — when those installers ship, use cargo-packager's native
  cross-platform updater (signer + endpoints), a separate mechanism from AppImage zsync.
