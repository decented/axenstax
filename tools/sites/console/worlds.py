"""Served-world manifest — `<worlds>/.identity/worlds.json`.

The console's record of the worlds this server hosts. Publish flow build spec
(`docs/superpowers/specs/2026-06-22-publish-flow-build-spec.md` §2.2): each served
world is a slot under `<worlds>/<id>/` plus an entry here carrying its title,
kind (normal/gallery), access policy, content owner, and terrain preset.

The alpha MVP serves a single world (the engine's `AXENSTAX_WORLD` slot), so the
manifest usually holds one entry; the schema generalises to many when the
multi-world hub is built (a separate spec). Pure file I/O — no engine change, no
HTTP between containers (same posture as wizard.py / studio.py).

Entry schema (one per served world):
    id                 — the served-world slot id (folder name; MVP = AXENSTAX_WORLD)
    title              — display name
    kind               — "normal" | "gallery"
    access             — "open" | "signin" | "friends"
    content_owner_npub — operator npub that owns the world's content
    world_type         — "normal" | "flat"
    ground             — flat-world ground block
    water_depth        — flat "water" ground depth (blocks)
    game_mode          — engine game mode the slot runs in
    created_unix       — first-created timestamp (preserved across re-publishes)
    updated_unix       — last write
"""

import io
import json
import os
import re
import secrets
import shutil
import tarfile
import time
from pathlib import Path

import identity
import studio  # _ALLOWED_EXT / _looks_like_image — reused so a published .axeworld
                # can't smuggle a non-image (e.g. exhibits/x.html) into the exhibits
                # dir that Caddy serves same-origin with the admin cookie (2026-09-27 audit).

_MANIFEST_VERSION = 1

# A served-world slot id is a folder name on the volume — keep it to a safe,
# flat charset so an id can never traverse out of the worlds root.
_ID_RE = re.compile(r"^[A-Za-z0-9 ._-]{1,64}$")

# Decompressed-size ceiling for an incoming publish (galleries carry image
# bytes). Mirrors the engine's import-bomb posture; refuse rather than fill disk.
_MAX_PUBLISH_BYTES = 256 * 1024 * 1024
# Raw (compressed) upload ceiling — the .axeworld is gzipped, so raw ≤ decompressed.
MAX_PUBLISH_UPLOAD_BYTES = 128 * 1024 * 1024
# Per-exhibit-image cap (mirrors the engine's pack-time limit) — applied on export
# so a rogue oversized file under exhibits/ can't be streamed out.
_MAX_EXHIBIT_IMAGE_BYTES = 12 * 1024 * 1024


def manifest_path() -> Path:
    return identity.identity_dir() / "worlds.json"


def read_manifest() -> dict:
    """The full manifest doc `{version, worlds: [...]}`, or an empty shell if the
    file is missing / unreadable / malformed (never raises)."""
    try:
        raw = manifest_path().read_text(encoding="utf-8")
    except OSError:
        return {"version": _MANIFEST_VERSION, "worlds": []}
    try:
        doc = json.loads(raw)
    except json.JSONDecodeError:
        return {"version": _MANIFEST_VERSION, "worlds": []}
    if not isinstance(doc, dict) or not isinstance(doc.get("worlds"), list):
        return {"version": _MANIFEST_VERSION, "worlds": []}
    return doc


def list_worlds() -> list[dict]:
    """All served-world entries (display order = manifest order)."""
    return list(read_manifest().get("worlds", []))


def get_world(world_id: str) -> dict | None:
    for w in list_worlds():
        if w.get("id") == world_id:
            return w
    return None


def _write_manifest(doc: dict) -> None:
    p = manifest_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(doc, indent=2), encoding="utf-8")


def record_world(entry: dict) -> dict:
    """Upsert a served-world entry by `id`. Preserves the original `created_unix`
    on an update (a re-publish overwrites content, not the world's birth date) and
    stamps `updated_unix`. Returns the stored entry."""
    world_id = str(entry.get("id") or "").strip()
    if not world_id:
        raise ValueError("world entry needs an id")

    doc = read_manifest()
    worlds = doc.setdefault("worlds", [])
    now = int(time.time())

    stored = dict(entry)
    stored["id"] = world_id
    stored["updated_unix"] = now

    for i, w in enumerate(worlds):
        if w.get("id") == world_id:
            stored["created_unix"] = w.get("created_unix") or now
            worlds[i] = stored
            break
    else:
        stored["created_unix"] = entry.get("created_unix") or now
        worlds.append(stored)

    doc["version"] = _MANIFEST_VERSION
    _write_manifest(doc)
    return stored


def is_valid_world_id(world_id: str) -> bool:
    """A served-world id must be a safe flat folder name (no traversal).

    Beyond the charset, the id has to be a *single real* path component. The
    charset allows `.`, so `"."` and `".."` slip through the regex — and pathlib
    collapses `<worlds>/.` back to the worlds root, which would make
    `world_slot_dir` (and everything downstream — pack/install) operate on the
    entire volume instead of one slot. `Path(".").parts` is `()` and
    `Path("..").parts` is `("..",)`, so requiring `parts == (id,)` rejects both
    while accepting every ordinary flat name."""
    wid = world_id or ""
    if not _ID_RE.match(wid) or ".." in wid:
        return False
    return Path(wid).parts == (wid,)


def world_slot_dir(world_id: str) -> Path:
    """On-disk folder for a served world: `<worlds>/<id>/`. (`identity_dir()` is
    `<worlds>/.identity`, so its parent is the worlds root.)"""
    return identity.identity_dir().parent / world_id


# The engine's host-only Proof-of-Play secret (`WorldMeta.pop_secret`, Spec 06
# §2.2). It must never leave the host in a shared archive, and a world arriving
# from outside must never keep the sender's secret.
_POP_SECRET_KEY = "pop_secret"


def _meta_without_secret(meta_bytes: bytes) -> bytes:
    """world_meta.json bytes with `pop_secret` removed (export path)."""
    meta = json.loads(meta_bytes.decode("utf-8"))
    if isinstance(meta, dict):
        meta.pop(_POP_SECRET_KEY, None)
    return json.dumps(meta).encode("utf-8")


def _meta_with_fresh_secret(meta_bytes: bytes) -> bytes:
    """world_meta.json bytes with any incoming `pop_secret` discarded and a fresh
    32-byte OS-RNG secret in its place (import path). Serialised the way serde
    writes a `[u8; 32]`: a JSON array of 32 integers."""
    meta = json.loads(meta_bytes.decode("utf-8"))
    if not isinstance(meta, dict):
        raise ValueError("world_meta.json is not an object")
    meta[_POP_SECRET_KEY] = list(secrets.token_bytes(32))
    return json.dumps(meta).encode("utf-8")


def pack_world_dir(world_id: str) -> bytes | None:
    """Pack a served world's on-disk folder into `.axeworld` bytes — gzip(tar) of
    `world_meta.json` + `world.dat` + `chunks/*` + `exhibits/*`, the exact member
    layout the engine's `world_archive` writes, so the game's `unpack_world` reads
    it (build spec §4, "pull to edit"). The engine stays HTTP-free; the console
    packs the folder at rest. Returns None if there's no such world on the volume.

    Tar headers are normalised (uid/gid/mtime zeroed) so the archive is
    deterministic and leaks no host metadata. Only the four known member kinds are
    included — nothing else in the folder travels."""
    if not is_valid_world_id(world_id):
        return None
    base = world_slot_dir(world_id)
    if not base.is_dir():
        return None

    def _norm(ti: tarfile.TarInfo) -> tarfile.TarInfo:
        ti.uid = ti.gid = 0
        ti.uname = ti.gname = ""
        ti.mtime = 0
        ti.mode = 0o644
        return ti

    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tf:
        meta_path = base / "world_meta.json"
        if meta_path.is_file():
            # An export is a share: strip the host-only PoP secret.
            try:
                meta_bytes = _meta_without_secret(meta_path.read_bytes())
            except (ValueError, UnicodeDecodeError):
                return None
            ti = _norm(tarfile.TarInfo("world_meta.json"))
            ti.size = len(meta_bytes)
            tf.addfile(ti, io.BytesIO(meta_bytes))
        dat_path = base / "world.dat"
        if dat_path.is_file():
            tf.add(dat_path, arcname="world.dat", filter=_norm)
        for sub in ("chunks", "exhibits"):
            d = base / sub
            if d.is_dir():
                for f in sorted(d.iterdir()):
                    if not f.is_file():
                        continue
                    # Skip a rogue oversized exhibit file rather than stream it out.
                    if sub == "exhibits" and f.stat().st_size > _MAX_EXHIBIT_IMAGE_BYTES:
                        continue
                    tf.add(f, arcname=f"{sub}/{f.name}", filter=_norm)
    return buf.getvalue()


def _extract_axeworld_members(blob: bytes) -> dict[str, bytes]:
    """Decompress + untar a `.axeworld` into an in-memory {member_path: bytes}
    map, accepting ONLY the engine's known member kinds and rejecting traversal /
    over-budget archives. Raises ValueError on a malformed or oversized archive."""
    out: dict[str, bytes] = {}
    total = 0
    try:
        tf = tarfile.open(fileobj=io.BytesIO(blob), mode="r:gz")
    except (tarfile.TarError, OSError) as e:
        raise ValueError(f"not a valid .axeworld archive: {e}") from e
    with tf:
        for m in tf.getmembers():
            if not m.isfile():
                continue
            name = m.name
            # Traversal guard (belt-and-braces over the allowlist below).
            if name.startswith("/") or ".." in name.split("/"):
                continue
            chunk_leaf = name[len("chunks/"):] if name.startswith("chunks/") else None
            exhibit_leaf = name[len("exhibits/"):] if name.startswith("exhibits/") else None
            # exhibits/* must also pass studio's image extension allowlist — 2026-09-27
            # audit fix. Without this, any file type (e.g. exhibits/x.html) survived
            # into the served-world slot, and Caddy's /exhibits route serves it
            # same-origin with the /admin/* console cookie (studio.save_image already
            # enforces this for direct uploads; a published .axeworld was the gap).
            exhibit_ext_ok = (
                exhibit_leaf is not None
                and Path(exhibit_leaf).suffix.lower() in studio._ALLOWED_EXT
            )
            allowed = (
                name in ("world_meta.json", "world.dat")
                or (chunk_leaf is not None and chunk_leaf.endswith(".chunk") and "/" not in chunk_leaf)
                or (exhibit_leaf is not None and exhibit_leaf and "/" not in exhibit_leaf and exhibit_ext_ok)
            )
            if not allowed:
                continue
            f = tf.extractfile(m)
            if f is None:
                continue
            # Cap on the bytes ACTUALLY read, not the header-declared size — a
            # crafted header could understate `m.size` to slip past the cap.
            data = f.read()
            # Magic-byte sniff on exhibits, same as studio.save_image — an
            # extension alone is trivially spoofed (rename x.html to x.png).
            if exhibit_leaf is not None and not studio._looks_like_image(data):
                continue
            total += len(data)
            if total > _MAX_PUBLISH_BYTES:
                raise ValueError("publish archive exceeds size cap")
            out[name] = data
    return out


def install_published_world(world_id: str, axeworld_bytes: bytes) -> tuple[bool, str | None]:
    """Unpack a published `.axeworld` into the served-world slot `<worlds>/<id>/`,
    replacing its contents (build spec §1, §7C). Only the known member kinds are
    written; a world is rejected unless it carries both `world_meta.json` and
    `world.dat`. The swap is two atomic renames (old → .bak, staging → slot) so the
    slot is never half-written. Returns (True, None) on success, else (False, err).

    The caller drops the `restart` sentinel so the entrypoint cycles the engine
    onto the new content; the autosave-vs-swap timing on a *running* engine is a
    live-systems detail validated at playtest (build spec §9)."""
    if not is_valid_world_id(world_id):
        return False, "bad-world-id"
    try:
        members = _extract_axeworld_members(axeworld_bytes)
    except ValueError as e:
        return False, str(e)
    if "world_meta.json" not in members or "world.dat" not in members:
        return False, "incomplete-archive (missing world_meta.json or world.dat)"
    # A world arriving from outside never keeps the sender's PoP secret.
    try:
        members["world_meta.json"] = _meta_with_fresh_secret(members["world_meta.json"])
    except (ValueError, UnicodeDecodeError):
        return False, "bad world_meta.json"

    slot = world_slot_dir(world_id)
    # Defence in depth: a world slot is always a direct child of the worlds
    # root. If a resolved slot ever sits anywhere else, the destructive
    # rename/rmtree below would operate on the volume rather than one world —
    # refuse rather than trust the id charset alone.
    worlds_root = identity.identity_dir().parent
    if slot.resolve().parent != worlds_root.resolve():
        return False, "bad-world-id"
    parent = slot.parent
    staging = parent / f"{world_id}.incoming"
    backup = parent / f"{world_id}.bak"

    shutil.rmtree(staging, ignore_errors=True)
    staging.mkdir(parents=True, exist_ok=True)
    for rel, data in members.items():
        dest = staging / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(data)

    shutil.rmtree(backup, ignore_errors=True)
    if slot.exists():
        os.replace(slot, backup)
    os.replace(staging, slot)
    shutil.rmtree(backup, ignore_errors=True)
    return True, None


def remove_world(world_id: str) -> bool:
    """Drop a world from the manifest. Returns True if an entry was removed.
    (Does NOT touch the on-disk world slot — that's the engine/entrypoint's job.)"""
    doc = read_manifest()
    worlds = doc.get("worlds", [])
    kept = [w for w in worlds if w.get("id") != world_id]
    if len(kept) == len(worlds):
        return False
    doc["worlds"] = kept
    _write_manifest(doc)
    return True
