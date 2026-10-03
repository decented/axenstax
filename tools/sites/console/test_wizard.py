#!/usr/bin/env python3
"""Stdlib tests for the setup-wizard logic (no pytest needed):

    python3 test_wizard.py

Each test runs against a throwaway identity dir so it never touches a real volume.
"""

import json
import os
import shlex
import sys
import tempfile
from pathlib import Path

_fails = 0


def check(cond, msg):
    global _fails
    if cond:
        print(f"  ok  — {msg}")
    else:
        _fails += 1
        print(f" FAIL — {msg}")


def fresh_env():
    """Point identity/world dirs at a brand-new temp tree and reset module state."""
    tmp = tempfile.mkdtemp(prefix="wiz-test-")
    os.environ["AXENSTAX_WORLDS_DIR"] = str(Path(tmp) / "worlds")
    os.environ.pop("AXENSTAX_IDENTITY_DIR", None)
    os.environ["AXENSTAX_WORLD"] = "server-world"
    os.environ.pop("AXENSTAX_SERVER_NAME", None)
    return Path(tmp) / "worlds"


def reimport():
    """Re-import the modules so any cached env is fresh (they read env per-call, but
    keep this defensive)."""
    for m in ("wizard", "studio", "identity"):
        sys.modules.pop(m, None)
    import identity, studio, wizard  # noqa: F401
    return wizard


def test_first_run():
    print("test_first_run")
    fresh_env()
    wizard = reimport()
    check(wizard.is_first_run(), "fresh server is first-run")
    check(wizard.read_state() is None, "no state yet")
    check(not wizard.engine_booted_once(), "engine gate not yet released")


def test_apply_gallery():
    print("test_apply_gallery")
    worlds = fresh_env()
    wizard = reimport()
    import identity, studio
    r = wizard.apply({
        "server_type": "gallery",
        "server_name": "My Gallery",
        "about": "art",
        "access": "open",
        "announce": True,
        "keep_history": False,
    })
    # Gallery runs as a CREATIVE world (operator builds) and starts in BUILD mode
    # (kiosk off); the operator opens it to visitors later from the dashboard.
    check(r["game_mode"] == "creative", "gallery → creative world (operator can build)")
    check(r["gallery"] is True, "gallery flag set")
    check(r["showcase"] is False, "gallery starts in build mode (kiosk off)")
    check(r["first_run"] is True and r["restart"] is False, "first run, no restart")
    check(studio.showcase_config()["enabled"] is False, "showcase.json starts disabled")
    check(wizard.is_gallery() is True, "is_gallery() true after apply")
    check(wizard.server_type() == "gallery", "server_type() reports gallery")
    check(identity.settings()["server_name"] == "My Gallery", "name saved to console.json")
    check(identity.settings()["announce"] is True, "announce saved")
    check(identity.require_signin() is False, "open access ⇒ no sign-in")
    env = (identity.identity_dir() / "server.env").read_text()
    check("AXENSTAX_GAMEMODE='creative'" in env or "AXENSTAX_GAMEMODE=creative" in env,
          "server.env carries creative gamemode")
    check("AXENSTAX_SHOWCASE='0'" in env or "AXENSTAX_SHOWCASE=0" in env,
          "server.env carries showcase off")
    check(wizard.engine_booted_once(), "gate marker dropped")
    check(wizard.setup_complete(), "state marked complete")
    check(not wizard.is_first_run(), "no longer first-run")


def test_gallery_open_toggle():
    print("test_gallery_open_toggle")
    fresh_env()
    wizard = reimport()
    import studio
    wizard.apply({"server_type": "gallery", "server_name": "G", "access": "open"})
    check(studio.showcase_config()["enabled"] is False, "starts in build mode")
    # "Open to visitors" = arm the kiosk (what /api/gallery/visitors does).
    studio.set_showcase_config(True, "board", 0)
    check(studio.showcase_config()["enabled"] is True, "open to visitors arms the kiosk")
    # back to build mode
    studio.set_showcase_config(False, "board", 0)
    check(studio.showcase_config()["enabled"] is False, "back to build disarms the kiosk")


def test_env_quoting():
    print("test_env_quoting")
    fresh_env()
    wizard = reimport()
    import identity
    wizard.apply({"server_type": "survival", "server_name": "Axe'n'Stax Server",
                  "access": "open"})
    envfile = identity.identity_dir() / "server.env"
    # Source the file the way the entrypoint does and confirm the tricky name parses.
    parsed = {}
    for line in envfile.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        k, _, v = line.partition("=")
        parsed[k] = shlex.split(v)[0] if v else ""
    check(parsed.get("AXENSTAX_SERVER_NAME") == "Axe'n'Stax Server",
          "apostrophe name round-trips through shell quoting")


def test_friends():
    print("test_friends")
    fresh_env()
    wizard = reimport()
    import identity
    good = identity.npub_encode("11" * 32)
    r = wizard.apply({
        "server_type": "survival",
        "server_name": "S",
        "access": "friends",
        "friends": f"{good}\nnot-an-npub\n{good}",  # dup + junk
    })
    check(identity.require_signin() is True, "friends ⇒ require sign-in")
    check(identity.allowlist() == [good], "only the valid npub is allowlisted, deduped")
    check(r["ok"], "apply ok")


def test_rerun_mode_change_resets_world():
    print("test_rerun_mode_change_resets_world")
    worlds = fresh_env()
    wizard = reimport()
    import identity
    wizard.apply({"server_type": "survival", "server_name": "S", "access": "open"})
    # Simulate an existing world on the volume.
    (worlds / "server-world").mkdir(parents=True, exist_ok=True)
    r = wizard.apply({"server_type": "creative", "server_name": "S", "access": "open"})
    check(r["fresh_world"] is True, "mode change with existing world ⇒ fresh world")
    check(r["restart"] is True, "re-run schedules a restart")
    check((identity.identity_dir() / "reset-world").exists(), "reset-world sentinel dropped")
    check((identity.identity_dir() / "restart").exists(), "restart sentinel dropped")


def test_rerun_same_mode_no_reset():
    print("test_rerun_same_mode_no_reset")
    worlds = fresh_env()
    wizard = reimport()
    import identity
    wizard.apply({"server_type": "survival", "server_name": "S", "access": "open"})
    (worlds / "server-world").mkdir(parents=True, exist_ok=True)
    r = wizard.apply({"server_type": "survival", "server_name": "S2", "access": "signin"})
    check(r["fresh_world"] is False, "same mode ⇒ no fresh world")
    check(r["restart"] is False, "same mode ⇒ no restart")
    check(not (identity.identity_dir() / "reset-world").exists(), "no reset-world sentinel")
    check(identity.settings()["server_name"] == "S2", "live rename applied")
    check(identity.require_signin() is True, "access change applied live")


def test_skip():
    print("test_skip")
    fresh_env()
    wizard = reimport()
    r = wizard.skip()
    check(wizard.engine_booted_once(), "skip releases the engine gate (boots defaults)")
    check(not wizard.setup_complete(), "skip does NOT mark setup complete")
    check(not wizard.is_first_run(), "skip records state so '/' stops forcing the wizard")
    check(wizard.read_state().get("skipped") is True, "skip flag recorded")


def test_apply_terrain_flat():
    print("test_apply_terrain_flat")
    fresh_env()
    wizard = reimport()
    import identity
    import worlds as worlds_mod
    owner = identity.npub_encode("22" * 32)
    r = wizard.apply({
        "server_type": "gallery",
        "server_name": "Flat Gallery",
        "access": "open",
        "world_type": "flat",
        "ground": "sand",
        "water_depth": 5,
        "content_owner_npub": owner,
    })
    check(r["world_type"] == "flat", "flat terrain returned")
    check(r["ground"] == "sand", "sand ground returned")
    check(r["water_depth"] == 5, "water depth returned")
    env = (identity.identity_dir() / "server.env").read_text()
    check("AXENSTAX_WORLD_TYPE='flat'" in env or "AXENSTAX_WORLD_TYPE=flat" in env,
          "server.env carries flat world_type")
    check("AXENSTAX_GROUND='sand'" in env or "AXENSTAX_GROUND=sand" in env,
          "server.env carries sand ground")
    check("AXENSTAX_WATER_DEPTH='5'" in env or "AXENSTAX_WATER_DEPTH=5" in env,
          "server.env carries water depth")
    # The served world is recorded in the manifest with owner + kind + terrain.
    ws = worlds_mod.list_worlds()
    check(len(ws) == 1, "manifest has one served world")
    check(ws[0]["kind"] == "gallery", "manifest records gallery kind")
    check(ws[0]["world_type"] == "flat" and ws[0]["ground"] == "sand",
          "manifest records terrain")
    check(ws[0]["content_owner_npub"] == owner, "manifest records content owner")


def test_rerun_terrain_change_resets_world():
    print("test_rerun_terrain_change_resets_world")
    worlds_dir = fresh_env()
    wizard = reimport()
    import identity
    wizard.apply({"server_type": "creative", "server_name": "G", "access": "open",
                  "world_type": "normal"})
    (worlds_dir / "server-world").mkdir(parents=True, exist_ok=True)
    r = wizard.apply({"server_type": "creative", "server_name": "G", "access": "open",
                      "world_type": "flat", "ground": "stone"})
    check(r["fresh_world"] is True, "terrain change with existing world ⇒ fresh world")
    check(r["restart"] is True, "terrain change schedules a restart")
    check((identity.identity_dir() / "reset-world").exists(), "reset-world sentinel dropped")


def test_invalid_terrain_falls_back_to_defaults():
    print("test_invalid_terrain_falls_back_to_defaults")
    fresh_env()
    wizard = reimport()
    r = wizard.apply({"server_type": "survival", "server_name": "S", "access": "open",
                      "world_type": "lavaland", "ground": "cheese", "water_depth": 999})
    check(r["world_type"] == "normal", "junk world_type ⇒ normal")
    check(r["ground"] == "grass", "junk ground ⇒ grass")
    check(r["water_depth"] == 32, "out-of-range depth clamped to 32")


def main():
    # Make local imports resolve when run from anywhere.
    sys.path.insert(0, str(Path(__file__).parent))
    for t in (test_first_run, test_apply_gallery, test_gallery_open_toggle, test_env_quoting,
              test_friends, test_rerun_mode_change_resets_world, test_rerun_same_mode_no_reset,
              test_apply_terrain_flat, test_rerun_terrain_change_resets_world,
              test_invalid_terrain_falls_back_to_defaults, test_skip):
        t()
    print()
    if _fails:
        print(f"{_fails} check(s) FAILED")
        sys.exit(1)
    print("all wizard checks passed")


if __name__ == "__main__":
    main()
