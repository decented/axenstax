"""Read + write the dedicated server's identity directory.

The Operator Console is HTTP-free in the engine (amendment B-0): it manages the
exact on-disk policy files the engine reloads every ~5s. This module is the
single source of truth for those files:

    <identity-dir>/
      attestation.json   operator-signed delegation event (operator npub = its pubkey)
      whitelist.txt      one npub per line (allowlist)
      blocklist.txt      one npub per line (blocklist; block wins)
      require_signin     "true" / "false" (absent = the engine default: sign-in required)
      console.json       ConsoleSettings (name/about/region/cap/announce/privacy)
      sessions.jsonl     operator-private session log (npub + timestamps, no IP/geo)
      kick               queued npubs to disconnect (consumed by the engine)

The identity dir defaults to `$AXENSTAX_IDENTITY_DIR`, else `$AXENSTAX_WORLDS_DIR/.identity`,
else `/worlds/.identity` (the Docker volume).

Includes a self-contained bech32 (BIP-173) so the console needs no crypto dep for
npub display/validation — `auth.py` owns the secp256k1 signature verification.
"""

import json
import os
import time
from pathlib import Path

# ─────────────────────────────── bech32 (BIP-173) ───────────────────────────────

_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"


def _bech32_polymod(values):
    gen = [0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3]
    chk = 1
    for v in values:
        top = chk >> 25
        chk = ((chk & 0x1FFFFFF) << 5) ^ v
        for i in range(5):
            chk ^= gen[i] if ((top >> i) & 1) else 0
    return chk


def _bech32_hrp_expand(hrp):
    return [ord(c) >> 5 for c in hrp] + [0] + [ord(c) & 31 for c in hrp]


def _bech32_create_checksum(hrp, data):
    values = _bech32_hrp_expand(hrp) + data
    polymod = _bech32_polymod(values + [0, 0, 0, 0, 0, 0]) ^ 1
    return [(polymod >> 5 * (5 - i)) & 31 for i in range(6)]


def _bech32_verify_checksum(hrp, data):
    return _bech32_polymod(_bech32_hrp_expand(hrp) + data) == 1


def _bech32_encode(hrp, data):
    combined = data + _bech32_create_checksum(hrp, data)
    return hrp + "1" + "".join(_CHARSET[d] for d in combined)


def _bech32_decode(bech):
    if any(ord(c) < 33 or ord(c) > 126 for c in bech):
        return None, None
    if bech.lower() != bech and bech.upper() != bech:
        return None, None
    bech = bech.lower()
    pos = bech.rfind("1")
    if pos < 1 or pos + 7 > len(bech) or len(bech) > 200:
        return None, None
    if not all(c in _CHARSET for c in bech[pos + 1 :]):
        return None, None
    hrp = bech[:pos]
    data = [_CHARSET.find(c) for c in bech[pos + 1 :]]
    if not _bech32_verify_checksum(hrp, data):
        return None, None
    return hrp, data[:-6]


def _convertbits(data, frombits, tobits, pad=True):
    acc = 0
    bits = 0
    ret = []
    maxv = (1 << tobits) - 1
    max_acc = (1 << (frombits + tobits - 1)) - 1
    for value in data:
        if value < 0 or (value >> frombits):
            return None
        acc = ((acc << frombits) | value) & max_acc
        bits += frombits
        while bits >= tobits:
            bits -= tobits
            ret.append((acc >> bits) & maxv)
    if pad:
        if bits:
            ret.append((acc << (tobits - bits)) & maxv)
    elif bits >= frombits or ((acc << (tobits - bits)) & maxv):
        return None
    return ret


def npub_encode(hex_pubkey: str) -> str | None:
    """32-byte hex x-only pubkey → npub bech32, or None if malformed."""
    try:
        raw = bytes.fromhex(hex_pubkey)
    except ValueError:
        return None
    if len(raw) != 32:
        return None
    data = _convertbits(list(raw), 8, 5, True)
    if data is None:
        return None
    return _bech32_encode("npub", data)


def npub_decode(npub: str) -> str | None:
    """npub bech32 → 32-byte hex x-only pubkey, or None if malformed / not an npub."""
    hrp, data = _bech32_decode(npub.strip())
    if hrp != "npub" or data is None:
        return None
    raw = _convertbits(data, 5, 8, False)
    if raw is None or len(raw) != 32:
        return None
    return bytes(raw).hex()


def npub_to_hex(npub: str) -> str | None:
    """npub bech32 → 64-hex, or None if not a valid npub."""
    hrp, data = _bech32_decode(npub)
    if hrp != "npub" or data is None:
        return None
    raw = _convertbits(data, 5, 8, False)
    if raw is None or len(raw) != 32:
        return None
    return bytes(raw).hex()


def is_valid_npub(s: str) -> bool:
    return bool(s) and npub_to_hex(s.strip()) is not None


# ─────────────────────────────── identity dir ───────────────────────────────


def identity_dir() -> Path:
    """The dedicated server's identity directory (the shared volume)."""
    explicit = os.environ.get("AXENSTAX_IDENTITY_DIR", "").strip()
    if explicit:
        return Path(explicit)
    worlds = os.environ.get("AXENSTAX_WORLDS_DIR", "/worlds").strip() or "/worlds"
    return Path(worlds) / ".identity"


def _read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError:
        return ""


def _read_npub_list(path: Path) -> list[str]:
    """Parse an npub-per-line file (skips blanks + `#` comments)."""
    out: list[str] = []
    for line in _read_text(path).splitlines():
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        out.append(s)
    return out


def _write_npub_list(path: Path, npubs: list[str]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(npubs) + ("\n" if npubs else ""), encoding="utf-8")


# ── identity / attestation ──


def attestation() -> dict | None:
    raw = _read_text(identity_dir() / "attestation.json")
    if not raw.strip():
        return None
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return None


def _tag_value(event: dict, name: str) -> str | None:
    for tag in event.get("tags", []):
        if isinstance(tag, list) and len(tag) >= 2 and tag[0] == name:
            return tag[1]
    return None


def operator_pubkey_hex() -> str | None:
    """The PRIMARY operator's pubkey (hex) — the signer of the attestation. `None`
    if the server isn't provisioned (no attestation)."""
    att = attestation()
    if not att:
        return None
    pk = att.get("pubkey")
    return pk.lower() if isinstance(pk, str) and len(pk) == 64 else None


def _read_pubkey_file(filename: str) -> set[str]:
    """Hex pubkeys from a `<identity_dir>/<filename>` list — one `npub…` (or 64-char
    hex) per line; blank lines and `#` comments (full-line AND inline) ignored."""
    out: set[str] = set()
    for line in _read_text(identity_dir() / filename).splitlines():
        s = line.split("#", 1)[0].strip()
        if not s:
            continue
        if len(s) == 64:
            try:
                bytes.fromhex(s)
                out.add(s.lower())
                continue
            except ValueError:
                pass
        h = npub_decode(s)
        if h:
            out.add(h)
    return out


def additional_operator_pubkeys_hex() -> set[str]:
    """Legacy `operators.txt` allowlist (pre-roles). Treated as **admins** by the
    role model below, for back-compat — new grants should use the role files."""
    return _read_pubkey_file("operators.txt")


# ── Roles (owner > admin > moderator) ──────────────────────────────────────────
# Role membership lives in `<identity_dir>/{owners,admins,moderators}.txt` (same
# format as the allow/block lists). The PRIMARY attestation operator is always an
# owner. A pubkey's effective role is the highest it appears in.

ROLE_FILES = {"owner": "owners.txt", "admin": "admins.txt", "moderator": "moderators.txt"}

# Capabilities each role grants. The dashboard hides any panel/action whose cap is
# absent (not greyed); every API route checks its cap. `manage_*` = the Team panel.
_ROLE_CAPS: dict[str, set[str]] = {
    "moderator": {"access"},  # allowlist / blocklist / kick — nothing else
    "admin": {
        "access", "require_signin", "settings", "privacy",
        "showcase", "gallery", "studio", "manage_moderators",
    },
    "owner": {
        "access", "require_signin", "settings", "privacy",
        "showcase", "gallery", "studio",
        "manage_moderators", "manage_admins", "manage_owners",
    },
}
ROLE_ORDER = ["owner", "admin", "moderator"]


def role_pubkeys_hex(role: str) -> set[str]:
    """Pubkeys (hex) explicitly assigned `role`. Owner also implicitly includes the
    primary attestation operator; admin also includes legacy `operators.txt`."""
    out = _read_pubkey_file(ROLE_FILES[role]) if role in ROLE_FILES else set()
    if role == "owner":
        p = operator_pubkey_hex()
        if p:
            out.add(p)
    if role == "admin":
        out |= additional_operator_pubkeys_hex()
    return out


def role_of(pubkey_hex: str | None) -> str | None:
    """The highest role a pubkey holds (owner > admin > moderator), or None."""
    if not pubkey_hex:
        return None
    pk = pubkey_hex.lower()
    for role in ROLE_ORDER:
        if pk in role_pubkeys_hex(role):
            return role
    return None


def caps_for(pubkey_hex: str | None) -> set[str]:
    """Capability set for a pubkey, derived from its role. Empty if no role."""
    r = role_of(pubkey_hex)
    return set(_ROLE_CAPS.get(r, set())) if r else set()


def operator_pubkeys_hex() -> set[str]:
    """Every pubkey allowed into the console — anyone holding ANY role (owner,
    admin, or moderator), plus the primary operator + legacy `operators.txt`.

    This is the "can sign in at all" set. Do NOT use it as the trusted-signer set
    for a capability-gated action — a moderator holds a role but only the
    `access` cap, so gating on membership here would let a moderator authorize
    owner/admin-only operations. Use `pubkeys_with_cap(cap)` for that."""
    out: set[str] = set()
    for role in ROLE_FILES:
        out |= role_pubkeys_hex(role)
    return out


def pubkeys_with_cap(cap: str) -> set[str]:
    """Every pubkey whose role grants capability `cap`. This is the trusted-signer
    set for a capability-gated action reached OUTSIDE the cookie session — e.g. a
    game-signed publish event, which can't go through `_require_cap` because it
    carries its own signature rather than a session cookie. Mirrors the cap the
    equivalent console-driven route checks, so a lower role can't authorize via
    the signed path what the web console reserves for a higher one."""
    out: set[str] = set()
    for role, caps in _ROLE_CAPS.items():
        if cap in caps:
            out |= role_pubkeys_hex(role)
    return out


def identity_summary() -> dict:
    """Operator/runtime npubs + delegation expiry for the dashboard header."""
    att = attestation()
    if not att:
        return {"provisioned": False}
    op_hex = att.get("pubkey")
    runtime_hex = _tag_value(att, "d")
    valid_until = _tag_value(att, "valid_until")
    name = _tag_value(att, "name")
    try:
        expires = int(valid_until) if valid_until else None
    except ValueError:
        expires = None
    return {
        "provisioned": True,
        "operator_npub": npub_encode(op_hex) if op_hex else None,
        "runtime_npub": npub_encode(runtime_hex) if runtime_hex else None,
        "server_name": name,
        "delegation_expires_unix": expires,
        "delegation_days_left": (
            max(0, int((expires - time.time()) / 86400)) if expires else None
        ),
    }


# ── access policy ──


def allowlist() -> list[str]:
    return _read_npub_list(identity_dir() / "whitelist.txt")


def blocklist() -> list[str]:
    return _read_npub_list(identity_dir() / "blocklist.txt")


def require_signin() -> bool:
    """The sign-in requirement the engine enforces. Mirrors
    `server_main::load_access_policy`: the file, when present, decides
    ("true" = required, anything else = guests admitted); with no file the
    dedicated server requires sign-in (owner decision 2026-10-06) unless it was
    started with `--allow-guests` / `AXENSTAX_ALLOW_GUESTS=1`, which the
    console can't see — so the console reports the default."""
    path = identity_dir() / "require_signin"
    if not path.exists():
        return True
    return _read_text(path).strip().lower() == "true"


def add_to_list(which: str, npub: str) -> None:
    """`which` is 'allow' | 'block'. Idempotent; validates the npub."""
    npub = npub.strip()
    if not is_valid_npub(npub):
        raise ValueError("not a valid npub")
    path = identity_dir() / ("whitelist.txt" if which == "allow" else "blocklist.txt")
    items = _read_npub_list(path)
    if npub not in items:
        items.append(npub)
        _write_npub_list(path, items)


def remove_from_list(which: str, npub: str) -> None:
    path = identity_dir() / ("whitelist.txt" if which == "allow" else "blocklist.txt")
    items = [n for n in _read_npub_list(path) if n != npub.strip()]
    _write_npub_list(path, items)


def set_require_signin(on: bool) -> None:
    d = identity_dir()
    d.mkdir(parents=True, exist_ok=True)
    (d / "require_signin").write_text("true" if on else "false", encoding="utf-8")


def queue_kick(npub: str) -> None:
    npub = npub.strip()
    if not is_valid_npub(npub):
        raise ValueError("not a valid npub")
    path = identity_dir() / "kick"
    existing = _read_text(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(existing + npub + "\n", encoding="utf-8")


# ── role membership (Team panel) ──


def _read_clean_npubs(path: Path) -> list[str]:
    """Role-file reader: one entry per line as `npub…` or 64-hex, with full-line
    AND inline `#` comments stripped; returns validated npubs (hex normalised to
    npub). Tolerant of the comments a hand-editor adds; the Team panel writes back
    clean (comments are a hand-edit convenience, not preserved across edits)."""
    out: list[str] = []
    for line in _read_text(path).splitlines():
        s = line.split("#", 1)[0].strip()
        if not s:
            continue
        if len(s) == 64:
            try:
                bytes.fromhex(s)
                s = npub_encode(s) or s
            except ValueError:
                pass
        if is_valid_npub(s):
            out.append(s)
    return out


def role_members(role: str) -> list[str]:
    """The npubs explicitly listed for `role` (Team panel). Excludes the implicit
    primary-operator owner and legacy `operators.txt` admins (those come from the
    attestation / the legacy file and aren't editable here)."""
    if role not in ROLE_FILES:
        return []
    return _read_clean_npubs(identity_dir() / ROLE_FILES[role])


def add_role_member(role: str, npub: str) -> None:
    """Grant `npub` the given role. A pubkey holds ONE role, so it is removed from
    the other role files first (promotion/demotion is a single call). Idempotent."""
    if role not in ROLE_FILES:
        raise ValueError("unknown role")
    npub = npub.strip()
    if not is_valid_npub(npub):
        raise ValueError("not a valid npub")
    hexpk = npub_decode(npub)
    for other in ROLE_FILES:
        if other == role:
            continue
        opath = identity_dir() / ROLE_FILES[other]
        cur = _read_clean_npubs(opath)
        kept = [n for n in cur if npub_decode(n) != hexpk]
        if len(kept) != len(cur):
            _write_npub_list(opath, kept)
    path = identity_dir() / ROLE_FILES[role]
    items = _read_clean_npubs(path)
    if not any(npub_decode(n) == hexpk for n in items):
        items.append(npub)
        _write_npub_list(path, items)


def remove_role_member(role: str, npub: str) -> None:
    """Revoke `npub`'s role membership from `role`'s file."""
    if role not in ROLE_FILES:
        raise ValueError("unknown role")
    path = identity_dir() / ROLE_FILES[role]
    hexpk = npub_decode(npub.strip())
    items = [n for n in _read_clean_npubs(path) if npub_decode(n) != hexpk]
    _write_npub_list(path, items)


# ── console settings (console.json) ──

_SETTINGS_DEFAULT = {
    "max_players": None,
    "announce": False,
    "server_name": None,
    "about": "",
    "region": "",
    "privacy_level": "",
    "privacy_retention_days": 0,
}


def settings() -> dict:
    raw = _read_text(identity_dir() / "console.json")
    if not raw.strip():
        return dict(_SETTINGS_DEFAULT)
    try:
        loaded = json.loads(raw)
    except json.JSONDecodeError:
        return dict(_SETTINGS_DEFAULT)
    merged = dict(_SETTINGS_DEFAULT)
    if isinstance(loaded, dict):
        merged.update({k: loaded[k] for k in _SETTINGS_DEFAULT if k in loaded})
    return merged


def save_settings(updates: dict) -> dict:
    """Apply a partial update to console.json (engine reads it on reload)."""
    s = settings()
    for k in _SETTINGS_DEFAULT:
        if k in updates:
            s[k] = updates[k]
    d = identity_dir()
    d.mkdir(parents=True, exist_ok=True)
    (d / "console.json").write_text(json.dumps(s, indent=2), encoding="utf-8")
    return s


# ── telemetry (sessions.jsonl) — operator-private; npub + timestamps only ──


def _sessions() -> list[dict]:
    out: list[dict] = []
    for line in _read_text(identity_dir() / "sessions.jsonl").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            out.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return out


def _save_sessions(sessions: list[dict]) -> None:
    d = identity_dir()
    d.mkdir(parents=True, exist_ok=True)
    (d / "sessions.jsonl").write_text(
        "".join(json.dumps(s) + "\n" for s in sessions), encoding="utf-8"
    )


def telemetry(now_unix: int | None = None, day_secs: int = 86400) -> dict:
    """Aggregates + the recent session list for the dashboard."""
    now = int(now_unix if now_unix is not None else time.time())
    day_start = now - day_secs
    sessions = _sessions()
    unique_today = {s.get("npub") for s in sessions if int(s.get("connect_unix", 0)) >= day_start}
    # peak concurrency over the window (open sessions end at `now`)
    events: list[tuple[int, int]] = []
    for s in sessions:
        start = int(s.get("connect_unix", 0))
        end = s.get("disconnect_unix")
        end = int(end) if end is not None else now
        if end < day_start or start > now:
            continue
        events.append((start, 1))
        events.append((end, -1))
    events.sort(key=lambda e: (e[0], -e[1]))
    cur = peak = 0
    for _, delta in events:
        cur += delta
        peak = max(peak, cur)
    recent = sorted(sessions, key=lambda s: int(s.get("connect_unix", 0)), reverse=True)[:50]
    return {
        "total_sessions": len(sessions),
        "unique_today": len([u for u in unique_today if u]),
        "peak_today": peak,
        "recent": recent,
    }


def forget_player(npub: str) -> None:
    npub = npub.strip()
    _save_sessions([s for s in _sessions() if s.get("npub") != npub])


def purge_history() -> None:
    _save_sessions([])
