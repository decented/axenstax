"""First-run server setup wizard — state + apply logic.

The Operator Console's WordPress/OS-style onboarding. On first sign-in the operator
picks "what kind of place is this?" (gallery / creative / survival / adventure) plus a
name, an access policy, and a couple of finishing toggles. This module turns those few
choices into the on-disk files the engine + web kiosk consume, and into the two
sentinels the dedicated-server entrypoint honours.

Two independent "setup done" signals (see the design doc
`docs/superpowers/specs/2026-06-21-server-setup-wizard-design.md`):

  • server.json (this module)      → the CONSOLE's record: "show the wizard?"
  • .identity/setup-complete       → the ENTRYPOINT's gate: "engine may boot"

Plus two one-shot sentinels the entrypoint drains:

  • .identity/restart      → cycle the engine (re-source server.env)
  • .identity/reset-world  → archive the world so it's recreated fresh in a new mode

Everything here is pure file I/O on the shared `<worlds>/.identity` volume — no engine
change, no HTTP between the containers. Reuses identity.py / studio.py for the files
they already own.
"""

import ipaddress
import json
import logging
import os
import re
import shlex
import time
from pathlib import Path

import identity
import studio
import worlds

log = logging.getLogger(__name__)

# ── server "type" → engine settings ─────────────────────────────────────────────
# The single big wizard choice. Each maps to a game mode (+ for a gallery, the kiosk
# capability). Keep this table the source of truth.
#
# Gallery deliberately runs as a CREATIVE world so the OPERATOR can build it and hang
# art — and "opening it to visitors" arms the web kiosk (showcase.json), which forces
# any web visitor to read-only regardless of the world mode (verified:
# game_loop.rs forces PlayMode::Adventure when showcase is on). A gallery therefore
# starts in BUILD mode (kiosk off) so the operator can go straight in and build, then
# flips to "open to visitors" from the dashboard. No world wipe, ever.
SERVER_TYPES = {
    "gallery":   {"game_mode": "creative", "gallery": True, "showcase_start": False,
                  "label": "Gallery",
                  "blurb": "Show off your builds. You build it; visitors look but can't change anything."},
    "creative":  {"game_mode": "creative", "gallery": False, "showcase_start": False,
                  "label": "Creative world", "blurb": "Everyone builds with unlimited blocks."},
    "survival":  {"game_mode": "survival", "gallery": False, "showcase_start": False,
                  "label": "Survival world", "blurb": "Gather, craft, and survive together."},
    "adventure": {"game_mode": "adventure", "gallery": False, "showcase_start": False,
                  "label": "Adventure", "blurb": "Explore and play, but don't change the world."},
}
DEFAULT_TYPE = "survival"

ACCESS_MODES = ("open", "signin", "friends")
# Sign-in by default (owner decision 2026-10-06): a dedicated server admits only
# verified Signet identities unless the operator explicitly opens it to guests.
DEFAULT_ACCESS = "signin"

# Terrain preset (Publish flow P1). "normal" = seeded biome terrain; "flat" = a
# blank canvas at a fixed ground. The engine's dedicated-server path reads these
# off WorldMeta when it bootstraps a fresh world (server_main.rs → server.rs);
# named biomes beyond normal/flat are deferred (build spec §5.5, §9).
WORLD_TYPES = ("normal", "flat")
DEFAULT_WORLD_TYPE = "normal"
GROUNDS = ("grass", "sand", "stone", "dirt", "snow", "water", "none")
DEFAULT_GROUND = "grass"

_NAME_MAX = 60
_ABOUT_MAX = 200
_DEFAULT_RETENTION_DAYS = 7


# ── paths ────────────────────────────────────────────────────────────────────────


def _id() -> Path:
    return identity.identity_dir()


def server_json_path() -> Path:
    return _id() / "server.json"


def server_env_path() -> Path:
    return _id() / "server.env"


def engine_marker_path() -> Path:
    return _id() / "setup-complete"


def restart_path() -> Path:
    return _id() / "restart"


def reset_world_path() -> Path:
    return _id() / "reset-world"


def awaiting_public_host_path() -> Path:
    return _id() / "awaiting-public-host"


def awaiting_public_host() -> bool:
    """The entrypoint is holding the engine back: nothing has told it the address
    players use, and a server with a WebSocket listener refuses to boot without one."""
    return awaiting_public_host_path().exists()


# ── state ──────────────────────────────────────────────────────────────────────


def read_state() -> dict | None:
    """The console's record of the last wizard run, or None if never run."""
    try:
        raw = server_json_path().read_text(encoding="utf-8")
    except OSError:
        return None
    if not raw.strip():
        return None
    try:
        d = json.loads(raw)
        return d if isinstance(d, dict) else None
    except json.JSONDecodeError:
        return None


def is_first_run() -> bool:
    """True until the wizard has been completed OR explicitly skipped once."""
    return read_state() is None


def setup_complete() -> bool:
    s = read_state()
    return bool(s and s.get("setup_complete"))


def server_type() -> str:
    """The chosen 'kind of place' (gallery/creative/survival/adventure)."""
    s = read_state() or {}
    return s.get("server_type") or DEFAULT_TYPE


def is_gallery() -> bool:
    s = read_state() or {}
    return bool(s.get("gallery"))


def engine_booted_once() -> bool:
    """True once the engine gate has been released (wizard finished or skipped)."""
    return engine_marker_path().exists()


def world_exists() -> bool:
    return (studio._worlds_dir() / studio.world_name()).is_dir()


def _access_in_force() -> str:
    """Who the policy files admit right now, as a wizard answer. The allowlist can be
    changed on the dashboard after the wizard ran, so this reads the files, not the
    wizard's own record."""
    if identity.allowlist_enabled():
        return "friends"
    return "signin" if identity.require_signin() else "open"


def current_state_for_form() -> dict:
    """Defaults the wizard form renders from — prior choices if any, else sensible
    defaults seeded from the live settings the operator may already have set."""
    prev = read_state() or {}
    settings = identity.settings()
    name = prev.get("server_name") or settings.get("server_name") \
        or os.environ.get("AXENSTAX_SERVER_NAME") or "Axe'n'Stax Server"
    return {
        "server_type": prev.get("server_type") or DEFAULT_TYPE,
        "server_name": name,
        "about": prev.get("about") if prev.get("about") is not None else settings.get("about", ""),
        "access": _access_in_force(),
        "announce": bool(prev.get("announce", settings.get("announce", False))),
        "keep_history": bool(prev.get("keep_history",
                                      settings.get("privacy_level") == "sessions")),
        "friends": identity.allowlist(),
        "public_host": _read_env().get("AXENSTAX_PUBLIC_HOST", ""),
        "world_type": prev.get("world_type") or DEFAULT_WORLD_TYPE,
        "ground": prev.get("ground") or DEFAULT_GROUND,
        "water_depth": prev.get("water_depth") or 3,
        "setup_complete": bool(prev.get("setup_complete")),
        "skipped": bool(prev.get("skipped")),
        "world_exists": world_exists(),
        "engine_booted": engine_booted_once(),
        "types": SERVER_TYPES,
        "world_types": WORLD_TYPES,
        "grounds": GROUNDS,
    }


# ── validation ────────────────────────────────────────────────────────────────


def _clean_type(v) -> str:
    v = str(v or "").strip().lower()
    return v if v in SERVER_TYPES else DEFAULT_TYPE


def _clean_access(v) -> str:
    """A present answer must be one of the three: a typo or an empty value doesn't
    silently become "signin" and turn an allowlist off."""
    answer = str(v if v is not None else "").strip().lower()
    if answer not in ACCESS_MODES:
        raise ValueError("access must be one of: " + ", ".join(ACCESS_MODES))
    return answer


def _clean_world_type(v) -> str:
    v = str(v or "").strip().lower()
    return v if v in WORLD_TYPES else DEFAULT_WORLD_TYPE


def _clean_ground(v) -> str:
    v = str(v or "").strip().lower()
    return v if v in GROUNDS else DEFAULT_GROUND


def _clean_water_depth(v) -> int:
    try:
        n = int(v)
    except (TypeError, ValueError):
        return 3
    return max(1, min(n, 32))


def _clean_text(v, limit: int) -> str:
    """One line of text. Raises `ValueError` on a line break inside it: the form's
    inputs are single-line, and these values go into the line-based `server.env`."""
    text = str(v or "").strip()
    if "\n" in text or "\r" in text:
        raise ValueError("this must be a single line of text")
    return text[:limit]


def _clean_friends(v) -> list[str]:
    """Accept a list or a newline/comma string of npubs; keep only valid ones."""
    if isinstance(v, str):
        parts = [p for chunk in v.splitlines() for p in chunk.split(",")]
    elif isinstance(v, (list, tuple)):
        parts = list(v)
    else:
        parts = []
    out: list[str] = []
    for p in parts:
        s = str(p).strip()
        if s and identity.is_valid_npub(s) and s not in out:
            out.append(s)
    return out


_HOST_NAME = re.compile(r"[A-Za-z0-9_-]{1,63}(\.[A-Za-z0-9_-]{1,63})*\.?")


def _valid_public_host_entry(entry: str) -> bool:
    """One `host[:port]`, by the rules the engine's `PublicHosts::parse` applies at
    boot: a value it would refuse must be refused here, where the operator can fix
    it, rather than stop the server. (ASCII only: a name in another script goes in
    as its punycode.)"""
    if entry.startswith("["):
        inner, closed, after = entry[1:].partition("]")
        port = after[1:] if after.startswith(":") else (None if not after else "bad")
        host_ok = bool(closed) and _is_ipv6(inner)
    elif entry.count(":") > 1:
        return _is_ipv6(entry)
    else:
        name, colon, port = entry.partition(":")
        port = port if colon else None
        host_ok = len(name) <= 253 and bool(_HOST_NAME.fullmatch(name))
    if port is not None and not (port.isascii() and port.isdigit() and 0 < int(port) <= 65535):
        return False
    return host_ok


def _is_ipv6(text: str) -> bool:
    if "%" in text:  # A zone id only means something on this machine.
        return False
    try:
        ipaddress.IPv6Address(text)
    except ValueError:
        return False
    return True


def clean_public_host(v) -> str:
    """A comma list of the addresses players use, tidied. "" when none was given.
    Raises `ValueError` on anything malformed."""
    entries = [e.strip() for e in str(v or "").split(",")]
    entries = [e for e in entries if e]
    for entry in entries:
        if not _valid_public_host_entry(entry):
            raise ValueError(
                f"'{entry}' isn't an address: give a host name or IP, optionally with "
                ":port (no https:// or path), e.g. play.example.org or 192.168.1.20"
            )
    return ",".join(entries)


# ── env writer (sourced by entrypoint.sh) ──────────────────────────────────────


_ENV_NAME = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")


_ENV_PREFIX = re.compile(rb"[ \t]*(?:export[ \t]+)?")


def _parse_env_entry(lines: list[bytes], start: int) -> tuple[str, str, int] | None:
    """The `KEY=value` at `lines[start]` and the index after it. A quoted value may run
    over several lines, as the shell would read it. Indentation and a leading `export`
    are fine, as they are to the shell that sources the file. None when it isn't a
    setting."""
    first = lines[start][_ENV_PREFIX.match(lines[start]).end():]
    key, eq, _ = first.partition(b"=")
    if not eq or not _ENV_NAME.fullmatch(key.decode("ascii", "replace")):
        return None
    for end in range(start + 1, len(lines) + 1):
        text = b"\n".join([first] + lines[start + 1:end])
        try:
            parts = shlex.split(text.decode("utf-8").partition("=")[2])
        except UnicodeError:
            return None
        except ValueError:  # No closing quote yet: it may close on a later line.
            continue
        return key.decode("ascii"), parts[0] if parts else "", end
    return None


def _read_env(*, for_update: bool = False) -> dict:
    """What `_write_env` last wrote. It also has to get the console through a file it
    didn't write cleanly, because this is how the address gets set: a line that can't
    be understood is skipped with a warning naming its setting or line number (never
    its value), and the rest is kept. The next `_write_env` leaves that line out, which
    also lets the entrypoint source the file again.

    A file that can't be read at all reads as empty for showing the setup page. With
    `for_update` it raises `ValueError` (the API's 400) instead, because a write that
    started from nothing would drop every other setting in the file."""
    try:
        raw = server_env_path().read_bytes()
    except FileNotFoundError:
        return {}
    except OSError as e:
        if for_update:
            raise ValueError(
                f"couldn't read server.env, nothing was changed: {e.strerror or type(e).__name__}"
            ) from e
        return {}
    lines = raw.split(b"\n")
    out = {}
    i = 0
    while i < len(lines):
        line = lines[i]
        if not line or line.lstrip().startswith(b"#"):
            i += 1
            continue
        entry = _parse_env_entry(lines, i)
        if entry is None:
            named = _ENV_PREFIX.sub(b"", line, count=1).partition(b"=")[0]
            what = (f"the setting {named.decode('ascii')} on line {i + 1}"
                    if _ENV_NAME.fullmatch(named.decode("ascii", "replace")) else f"line {i + 1}")
            log.warning("server.env: can't understand %s; skipping it", what)
            i += 1
            continue
        key, value, i = entry
        out[key] = value
    return out


def _write_env(pairs: dict) -> None:
    """Write shell-quoted `KEY=value` lines the entrypoint sources. shlex.quote keeps
    a name like  Axe'n'Stax Server  from breaking the shell parse."""
    lines = [
        "# Written by the Operator Console setup wizard — sourced by entrypoint.sh.",
        "# These AXENSTAX_* values override the compose defaults at engine boot.",
    ]
    for k, v in pairs.items():
        lines.append(f"{k}={shlex.quote(str(v))}")
    identity._write_atomic(server_env_path(), "\n".join(lines) + "\n")


def set_public_host(v) -> dict:
    """Set the address players use, changing nothing else. The engine reads it at
    boot, so this cycles it; an engine still waiting for one just starts."""
    public_host = clean_public_host(v)
    if not public_host:
        raise ValueError("give the address players use to reach this server")
    _write_env({**_read_env(for_update=True), "AXENSTAX_PUBLIC_HOST": public_host})
    request_restart()
    return {"ok": True, "public_host": public_host}


# ── apply ──────────────────────────────────────────────────────────────────────


def apply(choices: dict) -> dict:
    """Persist a completed wizard. `choices` is the (untrusted) form payload.

    Returns a summary incl. whether the engine will (re)boot and whether the world
    was scheduled for a fresh start, so the UI can tell the operator what happens next.
    """
    # First, so a bad address leaves everything as it was.
    public_host = clean_public_host(choices.get("public_host"))
    prior_env = _read_env(for_update=True)
    prior_public_host = prior_env.get("AXENSTAX_PUBLIC_HOST", "")
    # A run that names none ("Add a world") keeps the one the server has.
    public_host = public_host or prior_public_host

    stype = _clean_type(choices.get("server_type"))
    spec = SERVER_TYPES[stype]
    game_mode = spec["game_mode"]
    is_gallery = bool(spec.get("gallery"))
    # A gallery starts in BUILD mode (kiosk off) so the operator can build first; they
    # "open it to visitors" later from the dashboard. Non-galleries never arm the kiosk.
    showcase_on = bool(spec.get("showcase_start", False))

    name = _clean_text(choices.get("server_name"), _NAME_MAX) or "Axe'n'Stax Server"
    about = _clean_text(choices.get("about"), _ABOUT_MAX)
    # "Add a world" sends no `access`: it changes who may join in no way.
    choosing_access = "access" in choices
    access = _clean_access(choices.get("access")) if choosing_access else _access_in_force()
    announce = bool(choices.get("announce"))
    keep_history = bool(choices.get("keep_history"))
    friends = _clean_friends(choices.get("friends")) if choosing_access and access == "friends" else []
    force_fresh = bool(choices.get("start_fresh_world"))

    world_type = _clean_world_type(choices.get("world_type"))
    ground = _clean_ground(choices.get("ground"))
    water_depth = _clean_water_depth(choices.get("water_depth"))
    # Operator npub that owns this world's content (recorded in the manifest). The
    # endpoint injects it from the signed-in session; None if not supplied.
    content_owner_npub = choices.get("content_owner_npub") or None

    require_signin = access in ("signin", "friends")

    prev = read_state() or {}
    first_run = not engine_booted_once()
    prev_mode = prev.get("game_mode")
    # On a RE-RUN, changing the game mode OR the terrain needs a brand-new world
    # (both are baked into world metadata at creation; the dedicated server only
    # applies them when it bootstraps a fresh world). On FIRST run with an existing
    # world that doesn't match, the operator can opt to start fresh.
    mode_changed = (prev_mode is not None and prev_mode != game_mode)
    terrain_changed = prev.get("world_type") is not None and (
        prev.get("world_type") != world_type
        or prev.get("ground") != ground
        or prev.get("water_depth") != water_depth
    )
    needs_fresh = force_fresh or ((mode_changed or terrain_changed) and world_exists())

    # 1. Live settings the engine reloads (~5s) ─ name/about/announce/privacy.
    identity.save_settings({
        "server_name": name,
        "about": about,
        "announce": announce,
        "privacy_level": "sessions" if keep_history else "none",
        "privacy_retention_days": _DEFAULT_RETENTION_DAYS if keep_history else 0,
    })

    # 2. Access policy (live), only on an explicit answer.
    if choosing_access:
        identity.set_require_signin(require_signin)
        if access == "friends":
            # "Just my friends" with nobody named yet admits nobody, not everybody.
            identity.turn_allowlist_on()
        else:
            # Any other answer means it: the form opened on "friends" if a list was on.
            identity.allowlist_off()
    for npub in friends:
        try:
            identity.add_to_list("allow", npub)
        except ValueError:
            pass  # _clean_friends already validated; defensive

    # 3. Showcase / kiosk (live for the web client).
    studio.set_showcase_config(showcase_on, "board", 0)

    # 4. Engine boot config (server.env) — applies on the next (fresh) boot.
    _write_env({
        "AXENSTAX_GAMEMODE": game_mode,
        "AXENSTAX_SHOWCASE": "1" if showcase_on else "0",
        "AXENSTAX_EXIT_ACTION": "board",
        "AXENSTAX_AUTO_LOOP_SECS": "0",
        "AXENSTAX_SERVER_NAME": name,
        # The engine requires sign-in unless told otherwise; `open` is the only
        # choice that admits guests. (The `require_signin` file written above
        # is what the engine actually reads first; this keeps a fresh boot in
        # step with it. With no answer, the setting stays as it was.)
        **({"AXENSTAX_ALLOW_GUESTS": "0" if require_signin else "1"} if choosing_access
           else {k: v for k, v in prior_env.items() if k == "AXENSTAX_ALLOW_GUESTS"}),
        "AXENSTAX_ANNOUNCE": "1" if announce else "0",
        "AXENSTAX_WORLD_TYPE": world_type,
        "AXENSTAX_GROUND": ground,
        "AXENSTAX_WATER_DEPTH": str(water_depth),
        **({"AXENSTAX_PUBLIC_HOST": public_host} if public_host else {}),
    })

    # 5. The console's own record.
    state = {
        "setup_complete": True,
        "setup_version": 1,
        "server_type": stype,
        "game_mode": game_mode,
        "gallery": is_gallery,
        "server_name": name,
        "about": about,
        "access": access,
        "announce": announce,
        "keep_history": keep_history,
        "showcase": showcase_on,
        "world_type": world_type,
        "ground": ground,
        "water_depth": water_depth,
        "content_owner_npub": content_owner_npub,
        "skipped": False,
        "created_at": prev.get("created_at") or int(time.time()),
        "updated_at": int(time.time()),
    }
    save_state(state)

    # Record this served world in the manifest (build spec §2.2). MVP serves a
    # single slot keyed by the engine's AXENSTAX_WORLD name. The manifest is a
    # record, never a gate — a write failure must not block the engine config.
    try:
        worlds.record_world({
            "id": studio.world_name(),
            "title": name,
            "kind": "gallery" if is_gallery else "normal",
            "access": access,
            "content_owner_npub": content_owner_npub,
            "world_type": world_type,
            "ground": ground,
            "water_depth": water_depth,
            "game_mode": game_mode,
        })
    except Exception:
        pass

    # 6. Release the engine gate (first run) / schedule a restart (re-run).
    mark_engine_ready()
    restart_requested = False
    if needs_fresh:
        reset_world_path().write_text("", encoding="utf-8")
    address_changed = public_host != prior_public_host
    if not first_run and (needs_fresh or mode_changed or terrain_changed or address_changed):
        request_restart()
        restart_requested = True

    return {
        "ok": True,
        "server_type": stype,
        "game_mode": game_mode,
        "gallery": is_gallery,
        "showcase": showcase_on,
        "world_type": world_type,
        "ground": ground,
        "water_depth": water_depth,
        "first_run": first_run,
        "fresh_world": bool(needs_fresh),
        "restart": restart_requested,
        # The engine boots within a few seconds of the gate release / restart.
        "server_booting": first_run or restart_requested,
    }


def save_state(state: dict) -> None:
    p = server_json_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(state, indent=2), encoding="utf-8")


def mark_engine_ready() -> None:
    """Drop the entrypoint gate marker so the engine may boot."""
    p = engine_marker_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    if not p.exists():
        p.write_text(str(int(time.time())), encoding="utf-8")


def request_restart() -> None:
    p = restart_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text("", encoding="utf-8")


def skip() -> dict:
    """Operator chose to skip the wizard (small-text link). The box boots on the
    compose defaults, but the dashboard keeps nudging until setup is finished. With
    no address in compose the engine waits for one, and the dashboard asks for it."""
    mark_engine_ready()
    prev = read_state() or {}
    state = {
        "setup_complete": False,
        "skipped": True,
        "setup_version": 1,
        "created_at": prev.get("created_at") or int(time.time()),
        "updated_at": int(time.time()),
    }
    # Preserve any prior real choices so the form re-renders them.
    for k in ("server_type", "game_mode", "server_name", "about", "access",
              "announce", "keep_history", "showcase"):
        if k in prev:
            state[k] = prev[k]
    save_state(state)
    return {"ok": True, "first_run": not engine_booted_once()}


def reset() -> dict:
    """Operator wants to run the wizard again. Flip the console's record so `/` routes
    to /setup. The engine keeps running on its current config until the re-run applies."""
    prev = read_state() or {}
    prev["setup_complete"] = False
    prev["updated_at"] = int(time.time())
    save_state(prev)
    return {"ok": True}
