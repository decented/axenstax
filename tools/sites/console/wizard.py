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

import json
import os
import shlex
import time
from pathlib import Path

import identity
import studio
import worlds

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
DEFAULT_ACCESS = "open"

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
        "access": prev.get("access") or (
            "signin" if identity.require_signin() else DEFAULT_ACCESS),
        "announce": bool(prev.get("announce", settings.get("announce", False))),
        "keep_history": bool(prev.get("keep_history",
                                      settings.get("privacy_level") == "sessions")),
        "friends": identity.allowlist(),
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
    v = str(v or "").strip().lower()
    return v if v in ACCESS_MODES else DEFAULT_ACCESS


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
    return str(v or "").strip()[:limit]


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


# ── env writer (sourced by entrypoint.sh) ──────────────────────────────────────


def _write_env(pairs: dict) -> None:
    """Write shell-quoted `KEY=value` lines the entrypoint sources. shlex.quote keeps
    a name like  Axe'n'Stax Server  from breaking the shell parse."""
    lines = [
        "# Written by the Operator Console setup wizard — sourced by entrypoint.sh.",
        "# These AXENSTAX_* values override the compose defaults at engine boot.",
    ]
    for k, v in pairs.items():
        lines.append(f"{k}={shlex.quote(str(v))}")
    p = server_env_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text("\n".join(lines) + "\n", encoding="utf-8")


# ── apply ──────────────────────────────────────────────────────────────────────


def apply(choices: dict) -> dict:
    """Persist a completed wizard. `choices` is the (untrusted) form payload.

    Returns a summary incl. whether the engine will (re)boot and whether the world
    was scheduled for a fresh start, so the UI can tell the operator what happens next.
    """
    stype = _clean_type(choices.get("server_type"))
    spec = SERVER_TYPES[stype]
    game_mode = spec["game_mode"]
    is_gallery = bool(spec.get("gallery"))
    # A gallery starts in BUILD mode (kiosk off) so the operator can build first; they
    # "open it to visitors" later from the dashboard. Non-galleries never arm the kiosk.
    showcase_on = bool(spec.get("showcase_start", False))

    name = _clean_text(choices.get("server_name"), _NAME_MAX) or "Axe'n'Stax Server"
    about = _clean_text(choices.get("about"), _ABOUT_MAX)
    access = _clean_access(choices.get("access"))
    announce = bool(choices.get("announce"))
    keep_history = bool(choices.get("keep_history"))
    friends = _clean_friends(choices.get("friends")) if access == "friends" else []
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

    # 2. Access policy (live).
    identity.set_require_signin(require_signin)
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
        "AXENSTAX_REQUIRE_SIGNIN": "1" if require_signin else "0",
        "AXENSTAX_ANNOUNCE": "1" if announce else "0",
        "AXENSTAX_WORLD_TYPE": world_type,
        "AXENSTAX_GROUND": ground,
        "AXENSTAX_WATER_DEPTH": str(water_depth),
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
    if not first_run and (needs_fresh or mode_changed or terrain_changed):
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
    compose defaults, but the dashboard keeps nudging until setup is finished."""
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
