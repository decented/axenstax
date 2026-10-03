"""Studio — exhibit-image management for the Creator Gallery (Phase 3, console side).

Uploads / lists / deletes the served world's `exhibits/*.png|jpg` on the shared
worlds volume — exactly the files the dedicated server serves at `/exhibits/<ref>`
(the Caddy route) and the engine renders for exhibits placed in-world with
`/exhibit`. The cloud layers (Stash pull/push, Beacon publish/adopt) wire on top of
this later. Pure stdlib — no image deps; validation is by extension + magic bytes.
"""

import json
import os
from pathlib import Path

import identity  # for identity_dir() — where showcase.json lives (served by Caddy)

_ALLOWED_EXT = {".png", ".jpg", ".jpeg", ".gif", ".webp"}
_MAX_BYTES = 12 * 1024 * 1024  # 12 MiB per image
_MAGIC = (
    b"\x89PNG\r\n\x1a\n",  # png
    b"\xff\xd8\xff",        # jpg
    b"GIF87a",
    b"GIF89a",
    b"RIFF",               # webp (RIFF....WEBP)
)


def world_name() -> str:
    """The served world's folder name (mirrors the engine's AXENSTAX_WORLD)."""
    return (os.environ.get("AXENSTAX_WORLD", "") or "server-world").strip() or "server-world"


def _worlds_dir() -> Path:
    return Path(os.environ.get("AXENSTAX_WORLDS_DIR", "/worlds").strip() or "/worlds")


def exhibits_dir() -> Path:
    """`<worlds>/<world>/exhibits` — the dir the /exhibits Caddy route serves from."""
    return _worlds_dir() / world_name() / "exhibits"


def sanitize_image_ref(name: str) -> str:
    """Flat, traversal-safe filename keeping its extension (mirrors the engine's
    `save::sanitize_image_ref`). Returns '' if nothing safe survives."""
    base = os.path.basename(str(name))
    cleaned = "".join(c for c in base if c.isalnum() or c in "_.-")
    if not cleaned or ".." in cleaned:
        return ""
    return cleaned


def _looks_like_image(data: bytes) -> bool:
    return any(data.startswith(sig) for sig in _MAGIC)


def image_dimensions(data: bytes) -> tuple[int, int] | None:
    """Pixel (width, height) read from the image header — PNG / JPEG / GIF / WebP —
    using only the stdlib (no PIL, matching this module's no-deps posture). Returns
    None if the format/header can't be parsed; callers treat that as 'unknown' (the
    engine fits the aspect at render time regardless, so dims are an artist hint, not
    load-bearing)."""
    try:
        # PNG: 8-byte signature, then IHDR chunk; width/height are big-endian u32 at
        # offsets 16 and 20.
        if data.startswith(b"\x89PNG\r\n\x1a\n") and len(data) >= 24:
            w = int.from_bytes(data[16:20], "big")
            h = int.from_bytes(data[20:24], "big")
            return (w, h) if w > 0 and h > 0 else None
        # GIF: logical-screen width/height are little-endian u16 at offsets 6 and 8.
        if data[:6] in (b"GIF87a", b"GIF89a") and len(data) >= 10:
            w = int.from_bytes(data[6:8], "little")
            h = int.from_bytes(data[8:10], "little")
            return (w, h) if w > 0 and h > 0 else None
        # JPEG: scan the marker segments for a Start-Of-Frame (SOF0..SOF15, skipping
        # the non-SOF markers C4/C8/CC); height/width are the two u16 after the
        # 2-byte length + 1-byte precision.
        if data.startswith(b"\xff\xd8") and len(data) > 4:
            i = 2
            n = len(data)
            while i + 9 < n:
                if data[i] != 0xFF:
                    i += 1
                    continue
                marker = data[i + 1]
                if 0xC0 <= marker <= 0xCF and marker not in (0xC4, 0xC8, 0xCC):
                    h = int.from_bytes(data[i + 5 : i + 7], "big")
                    w = int.from_bytes(data[i + 7 : i + 9], "big")
                    return (w, h) if w > 0 and h > 0 else None
                if marker in (0xD8, 0xD9) or 0xD0 <= marker <= 0xD7:
                    i += 2  # standalone markers carry no length
                    continue
                seg_len = int.from_bytes(data[i + 2 : i + 4], "big")
                if seg_len < 2:
                    break
                i += 2 + seg_len
            return None
        # WebP: 'RIFF'....'WEBP' then a chunk. VP8X carries 24-bit (w-1,h-1); the
        # simple lossy 'VP8 ' header carries 14-bit w/h after the start code.
        if data[:4] == b"RIFF" and data[8:12] == b"WEBP" and len(data) >= 30:
            fourcc = data[12:16]
            if fourcc == b"VP8X":
                w = int.from_bytes(data[24:27], "little") + 1
                h = int.from_bytes(data[27:30], "little") + 1
                return (w, h)
            if fourcc == b"VP8 ":
                w = int.from_bytes(data[26:28], "little") & 0x3FFF
                h = int.from_bytes(data[28:30], "little") & 0x3FFF
                return (w, h) if w > 0 and h > 0 else None
            return None
    except (IndexError, ValueError):
        return None
    return None


def suggested_exhibit_command(name: str, dims: tuple[int, int] | None) -> str:
    """A ready-to-paste `/exhibit place` command sized to the image's true aspect,
    so the in-world frame matches the picture with no wasted space. Defaults the
    on-wall WIDTH to 4 blocks and derives the height from the pixel ratio. Falls
    back to a plain square box when dims are unknown (the engine still fits the
    aspect once it decodes the image)."""
    default_w = 4.0
    if dims and dims[0] > 0 and dims[1] > 0:
        h = round(default_w * dims[1] / dims[0], 2)
        return f"/exhibit place {name} wall {default_w:g} {h:g}"
    return f"/exhibit place {name} wall"


def list_images() -> list[dict]:
    d = exhibits_dir()
    if not d.is_dir():
        return []
    out: list[dict] = []
    for p in sorted(d.iterdir()):
        if p.is_file() and p.suffix.lower() in _ALLOWED_EXT:
            try:
                dims = image_dimensions(p.read_bytes())
            except OSError:
                dims = None
            entry: dict = {"name": p.name, "size": p.stat().st_size}
            if dims:
                entry["width"], entry["height"] = dims
                entry["aspect"] = round(dims[0] / dims[1], 3) if dims[1] else None
            entry["place_command"] = suggested_exhibit_command(p.name, dims)
            out.append(entry)
    return out


def save_image(filename: str, data: bytes) -> str:
    """Validate (name, extension, size, magic bytes) and write into the world's
    exhibits dir. Returns the stored filename. Raises ValueError on rejection."""
    name = sanitize_image_ref(filename)
    if not name:
        raise ValueError("bad filename")
    if Path(name).suffix.lower() not in _ALLOWED_EXT:
        raise ValueError("unsupported type — use PNG / JPG / GIF / WebP")
    if not data:
        raise ValueError("empty file")
    if len(data) > _MAX_BYTES:
        raise ValueError("too large (max 12 MiB)")
    if not _looks_like_image(data):
        raise ValueError("not a valid image (failed magic-byte check)")
    d = exhibits_dir()
    d.mkdir(parents=True, exist_ok=True)
    (d / name).write_bytes(data)
    return name


def delete_image(filename: str) -> None:
    name = sanitize_image_ref(filename)
    if not name:
        raise ValueError("bad filename")
    p = exhibits_dir() / name
    if p.is_file():
        p.unlink()


# ── showcase / kiosk config (live; served by Caddy at /showcase.json) ──


def _showcase_path() -> Path:
    # Lives in the identity dir, which Caddy serves at /showcase.json; the dedicated
    # page fetches it at boot to arm/disarm the kiosk.
    return identity.identity_dir() / "showcase.json"


def showcase_config() -> dict:
    try:
        d = json.loads(_showcase_path().read_text(encoding="utf-8"))
        if isinstance(d, dict):
            return {
                "enabled": bool(d.get("enabled")),
                "exit_action": str(d.get("exit_action") or "board"),
                "auto_loop_secs": int(d.get("auto_loop_secs") or 0),
            }
    except (OSError, ValueError):
        pass
    return {"enabled": False, "exit_action": "board", "auto_loop_secs": 0}


def set_showcase_config(enabled: bool, exit_action: str, auto_loop_secs) -> dict:
    try:
        secs = max(0, int(auto_loop_secs or 0))
    except (TypeError, ValueError):
        secs = 0
    cfg = {
        "enabled": bool(enabled),
        "exit_action": (str(exit_action or "board").strip().lower() or "board"),
        "auto_loop_secs": secs,
    }
    p = _showcase_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(cfg), encoding="utf-8")
    return cfg


def game_mode() -> str:
    return (os.environ.get("AXENSTAX_GAMEMODE", "") or "survival").strip() or "survival"


def world_status() -> dict:
    """Read-only world summary for the World panel."""
    return {"world": world_name(), "game_mode": game_mode(), "showcase": showcase_config()}


# ── exhibits list (the engine's exhibits.json sidecar) ──


def list_exhibits() -> list[dict]:
    """The world's placed exhibits — from the `exhibits.json` sidecar the engine
    writes next to `world.dat` on save (the console can't decode the bincode save).
    Empty if the world has none, or no sidecar has been written yet."""
    p = _worlds_dir() / world_name() / "exhibits.json"
    try:
        d = json.loads(p.read_text(encoding="utf-8"))
        if isinstance(d, list):
            return d
    except (OSError, ValueError):
        pass
    return []
