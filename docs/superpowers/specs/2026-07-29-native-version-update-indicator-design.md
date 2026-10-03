# Native version + update-available indicator

**Date:** 2026-07-29
**Status:** SHIPPED v0.2.18 (2026-07-30) — `update_check.rs` + `/download/latest.json` live; `check.sh` version-parity gate in place. Implementation deviates in one way: the result is a process-global (`update_check::state()`), not a per-menu `Receiver` — `MenuState::new()` re-runs every lobby return and a per-instance receiver would re-fire the request. In-place AppImage update added 2026-09-03 — see "In-place update" below.
**Applies to:** Linux AppImage (now), Android APK (when it ships)

## Problem

A native build gives the player no way to tell which version they are running, or
whether a newer one exists. On 2026-07-29 the owner sent a `/bug` from "the
native AppImage" and it never arrived — the installed build turned out to be from
**9 June**, a month older than the release that added the native mailbox. Nothing
in the running game said so, and nothing suggested an update existed.

The web build has no such problem: it is always current by construction. This is
a native-only gap.

## What the player sees

One line in the lobby header, directly beneath the AXE'N'STAX wordmark.

| Situation | Line | Clickable |
|---|---|---|
| Update available | `AxeNStax v0.2.16 (v0.2.18 available)` | yes → download page |
| Up to date | `AxeNStax v0.2.18 (latest)` | no |
| Offline / check failed / opted out | `AxeNStax v0.2.16` | no |
| Local build ahead of published | `AxeNStax v0.2.19 (dev)` | no |

The "(vX available)" fragment is the only clickable part; tapping it opens
`https://docs.axenstax.org/download` in the system browser.

## Architecture

### Client — `game/engine/src/update_check.rs` (native-only)

Follows the off-thread pattern already established by `native_file_dialog.rs` and
`native_signin.rs`, and for the same reason: a blocking network call on the
render thread would stall the frame loop.

- `spawn_check() -> Receiver<UpdateState>` starts a worker thread that performs a
  blocking `ureq` GET and sends exactly one result.
- The menu drains the receiver with `try_recv()` each frame — never blocks.
- Current version comes from `env!("CARGO_PKG_VERSION")`, already used by
  `wasm_feedback.rs`.

`ureq` + `rustls` are existing native dependencies and already cross-compile for
Android (both appear in the APK link line), so nothing is added to the graph.

```rust
pub enum UpdateState {
    /// Check disabled, in flight, or failed — show the bare version.
    Unknown,
    /// Running the newest published build.
    Current,
    /// A newer build exists; carries its version string, plus (2026-09-03) an
    /// optional AppImage reference for the in-place updater below — `None`
    /// on an older manifest or one with no Linux build yet.
    Available { version: String, appimage: Option<AppImageRef> },
    /// Local build is ahead of what is published (a dev/CI build).
    Ahead,
}
```

### Server — `GET /download/latest.json` in `tools/sites/docs/app.py`

Generated from the **same `_discover_installers()`** call that renders the
download page, so the advertised version cannot drift from the files actually
downloadable. Static, cacheable, no query parameters.

```json
{
  "version": "0.2.16",
  "linux_appimage": "axenstax-engine_0.2.16_x86_64.AppImage",
  "android_apk": null,
  "download_url": "https://docs.axenstax.org/download",
  "linux_appimage_sha256": "988cb00c22c80df26bb4c3a5281f8ab5e36cf00c006798a3593b41149457b794",
  "linux_appimage_url": "https://docs.axenstax.org/download/installer/axenstax-engine_0.2.16_x86_64.AppImage"
}
```

`android_apk` is `null` until the APK ships; the client must tolerate a null or
absent value rather than requiring it.

`linux_appimage_sha256` and `linux_appimage_url` (added 2026-09-03 for the
in-place updater) are what a future in-place AppImage updater needs to fetch
and verify the newest build without re-deriving anything itself:
`linux_appimage_url` is the versioned, user-facing
`/download/installer/{name}` route (not the bare `/download/{name}`
auto-update-channel route the `-latest-` AppImage's built-in updater uses).
Both are `null` whenever `linux_appimage` is.

## Data flow

```
launch ──► spawn worker ──GET /download/latest.json──► docs site
                                                            │
menu draws "AxeNStax v0.2.16"   ◄── Unknown (in flight) ─────┘
        │
        ▼ (next frame, result arrives)
menu draws "AxeNStax v0.2.16 (v0.2.18 available)"
```

## Privacy and the regulatory red lines

The check is a **plain GET of a static document**: no identifiers, no cookies, no
query string, no request body, no telemetry, and nothing is recorded per-user.
That keeps it unambiguously a version check rather than data collection, well
clear of red line #3 ("no central collection of kids' data").

It is on by default and disabled entirely by `AXENSTAX_NO_UPDATE_CHECK=1`, which
makes the line render as a bare version with no network call at all. A
self-hoster's build can therefore never touch AxeNStax infrastructure — which is
the point of shipping neutral, self-hostable software.

## Error handling

Every failure path is **silent** and collapses to `Unknown` (bare version, no
suffix): offline, DNS failure, timeout, non-200, malformed JSON, missing
`version` field. Logged at debug only.

- Timeout: 5s connect + read, so a dead network cannot leave a thread hanging.
- No retry. One attempt per launch; a stale reading for one session is harmless.
- No error dialog and no nag. A child on a train sees nothing unusual.

## Prerequisite: fix `packager.toml` first

`tools/packaging/packager.toml` is still `version = "0.2.15"` while the engine is
`0.2.16`, despite carrying the comment "keep in step with game/engine/Cargo.toml".
cargo-packager stamps artefact **names** from that file, so
`_discover_installers()` — and therefore this endpoint — would report `0.2.15`.

The first thing this feature would do on a current build is report `(dev)`: the
game believing it is *ahead* of the published release. The feature would be
correct and the release metadata wrong.

**Bump `packager.toml` in the same change**, and add a check.sh assertion that
the two version strings match so they cannot drift again.

## Testing

The comparison logic is where bugs live, so it is a pure function with unit
tests:

```rust
fn compare_versions(current: &str, latest: &str) -> UpdateState
```

Cases: equal → `Current`; patch/minor/major newer → `Available`; current newer →
`Ahead`; malformed either side → `Unknown`; differing segment counts
(`0.2` vs `0.2.1`); non-numeric segments; empty strings.

Plus a test asserting the endpoint's JSON keys match what the client parses —
client and server must agree, and nothing else enforces that.

The network call itself is not unit tested; it is a thin `ureq` wrapper whose
failure modes all collapse to `Unknown`.

## In-place update (2026-09-03)

**Status:** SHIPPED. `game/engine/src/self_update.rs` + the `linux_appimage_sha256`
/ `linux_appimage_url` fields added to `latest.json` (`tools/sites/docs/
versioning.py`).

Auto-download/auto-install (previously "Out of scope", below) turned out to be
worth building directly rather than via the cargo-packager `-latest-`/`.zsync`
updater channel it references: that channel trusts whatever is at a fixed
"latest" URL with no content hash, and the version indicator already knows
exactly which build it wants — reusing that knowledge to fetch and verify a
*named* file is a smaller, more auditable piece of code than wiring up a
third-party updater.

### What the player sees

When `UpdateState::Available` carries an AppImage reference AND this build IS
the AppImage currently running (`self_update::running_appimage()` — see
below), the version line grows an **"Update now"** button next to the existing
"(vX available)" link. Clicking it:

| Progress | Line |
|---|---|
| Downloading | `Downloading v0.2.19... 42%` (or `... 6.1 MiB` when the server sent no `Content-Length`) |
| Verifying | `Checking the download...` |
| Installed | `Updated to v0.2.19 —` + a **Restart** button |
| Failed | `Update failed: <short reason>` (the manual download link stays visible) |

The manual "(vX available)" link is never removed — it's the fallback for
every build this doesn't apply to (not an AppImage, or a manifest that
predates the sha256/url fields) and for a failed in-place attempt.

### AppImage gate

The AppImage runtime sets the env var `APPIMAGE` to the absolute path of the
`.AppImage` file it exec'd from before launching the embedded binary (also
`APPDIR`, unused here). `self_update::running_appimage()` reads it and
requires the path to name an existing regular file — a `.deb` install,
`cargo run`, or a bare-binary tarball extraction has no such env var and gets
`None`, so the button never appears there; those users keep the manual link.

### Flow

```
"Update now" clicked
  └─ self_update::start(running_path, AppImageRef, version)
       └─ worker thread:
            GET reference.url (30s timeout, no redirects)  ──► Progress::Downloading{received, total}
            stream body → "<dir>/.{filename}.part" in 1 MiB chunks
            Progress::Verifying
            sha256(part) == reference.sha256 (case-insensitive)?
              no  → remove .part, Progress::Failed(reason)   [running binary untouched]
              yes → fs::rename(part, target)                  [same dir ⇒ atomic]
                    Progress::Installed{version}
"Restart" clicked → self_update::relaunch(path) spawns a new process, caller exit(0)
```

### Safety

The running AppImage is **never** touched until the download is fully
verified: the new build is written to a sibling `.<filename>.part` file, and
only a `.part` whose sha256 matches the manifest is `rename`d over the
target. A same-directory `rename` is atomic on Linux, so there is no window
where the target is a truncated or partially-written file — a crash, kill,
or lost connection mid-download just leaves a stray `.part` (removed on the
next attempt's failure path) and the old binary running exactly as before.
Any failure at any stage (network, disk, hash mismatch) removes the `.part`
and reports `Failed` with a short reason; nothing auto-retries.

### Privacy

Unchanged from the version check above: a plain HTTPS GET of the URL named in
the signed release event (then, if that fails, the `latest.json` mirror URL).
No identifiers, no cookies, no telemetry, nothing recorded per-user.

### Integrity model — what this is and isn't

**Superseded 2026-09-27 (audit: "the signed Nostr release channel can be
bypassed").** The signed kind-30063 release event (`nostr_release.rs`, pinned
release key) is now the ONLY authority for the version, URL and sha256 of an
install — `update_check::decide`:

- signed event present → version, URL and sha256 all come from it; the HTTP
  manifest's `linux_appimage_url` is appended as a mirror (`AppImageRef.mirrors`),
  tried after the signed URL, and accepted only if its bytes hash to the SIGNED
  sha256 (`self_update::install_from_candidates`);
- no signed event → the HTTP version can only produce a notify-only
  `Available { appimage: None }` (download-page link, no install button);
- the filename must be a bare `[A-Za-z0-9._-]+` basename ending `.AppImage`
  (`update_check::is_valid_appimage_filename`), so `.<name>.part` always sits in
  the target AppImage's own directory; the sha256 is checked before the rename.
- a mirror URL from the unsigned manifest is only used if it is `https://`, and
  every download is capped at 512 MiB (`self_update::MAX_DOWNLOAD_BYTES`,
  streamed; exceeding it aborts and deletes the `.part`).

So a compromised docs host can no longer pick the installed bytes. See also `docs/spec/08-security-anti-cheat.md §1.2`'s note on the
download surface.

### Testing

Pure and unit-tested: `verify_sha256` (match, case-insensitive match,
mismatch, missing file), `install_over` (replaces target bytes, sets 0o755,
removes `.part`), `finish_install` (verify-then-rename ordering: a mismatch
leaves the OLD target bytes intact and removes the `.part`), and
`running_appimage` (env var absent, names a missing file, names a real file).
The network download itself is not unit-tested — same posture as
`fetch_latest_version` above — and `relaunch` is a two-line `Command::spawn`
wrapper not worth testing in-process.

### Out of scope (still)

- **Changelog rendering.** The link goes to the download page.
- **The web build and the Android APK.** Neither has a running AppImage to
  overwrite.
- **Nagging, forced updates, or auto-triggering the download.** The player
  must click "Update now"; nothing downloads without that click.
- **A code-signing / detached-signature scheme.** See "Integrity model" above.

## Release plan

1. Bump `game/engine/Cargo.toml` and `tools/packaging/packager.toml` together to
   the new version.
2. `./check.sh` must be ALL GREEN.
3. Merge to `main`, push (web deploys automatically).
4. `gh workflow run native-packages.yml --ref main -f linux_only=true`
5. `gh workflow run publish-installers.yml --ref main -f run_id=<id>`
6. Verify `https://docs.axenstax.org/download/latest.json` returns the new
   version, and the download page agrees.
