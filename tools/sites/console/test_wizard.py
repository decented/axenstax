#!/usr/bin/env python3
"""Stdlib tests for the setup-wizard logic (no pytest needed):

    python3 test_wizard.py

Each test runs against a throwaway identity dir so it never touches a real volume.
"""

import json
import os
import shlex
import subprocess
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
    env_text = (identity.identity_dir() / "server.env").read_text()
    check("AXENSTAX_ALLOW_GUESTS='1'" in env_text or "AXENSTAX_ALLOW_GUESTS=1" in env_text,
          "open access ⇒ server.env admits guests")
    check("AXENSTAX_REQUIRE_SIGNIN" not in env_text, "the retired env var is not written")
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


def test_signin_is_the_default():
    print("test_signin_is_the_default")
    fresh_env()
    wizard = reimport()
    import identity
    # A fresh box: no require_signin file yet ⇒ the engine default (required).
    check(identity.require_signin() is True, "no file ⇒ sign-in required (engine default)")
    check(wizard.DEFAULT_ACCESS == "signin", "the wizard preselects sign-in")
    check(wizard.current_state_for_form()["access"] == "signin", "fresh form shows sign-in")
    # The dashboard toggle turning sign-in off is honoured, and the form shows it.
    identity.set_require_signin(False)
    check(identity.require_signin() is False, "an explicit 'false' file admits guests")
    check(wizard.current_state_for_form()["access"] == "open", "form reflects the open toggle")
    # An explicit "signin" answer turns it back on and tells a fresh boot too.
    wizard.apply({"server_type": "survival", "server_name": "S", "access": "signin"})
    check(identity.require_signin() is True, "the signin answer requires sign-in")
    env = (identity.identity_dir() / "server.env").read_text()
    check("AXENSTAX_ALLOW_GUESTS='0'" in env or "AXENSTAX_ALLOW_GUESTS=0" in env,
          "sign-in ⇒ server.env does not admit guests")


def test_a_run_with_no_access_answer_changes_no_access():
    print("test_a_run_with_no_access_answer_changes_no_access")
    fresh_env()
    wizard = reimport()
    import identity
    path = identity.identity_dir() / "require_signin"

    # No answer on a fresh box: no file appears, and nothing says guests are in or out.
    wizard.apply({"server_type": "survival", "server_name": "S"})
    check(not path.exists(), "no require_signin file is written")
    check("AXENSTAX_ALLOW_GUESTS" not in env_of(identity), "and no guest setting either")

    # A server opened to guests stays open, and the records say so.
    identity.set_require_signin(False)
    wizard.apply({"server_type": "survival", "server_name": "S"})
    check(identity.require_signin() is False, "a guest-open server stays open")
    check(wizard.read_state()["access"] == "open", "the record says what is in force")

    # So does a server with an allowlist.
    identity.set_require_signin(True)
    identity.turn_allowlist_on()
    wizard.apply({"server_type": "survival", "server_name": "S"})
    check(identity.allowlist_enabled(), "the allowlist stays on")
    check(wizard.read_state()["access"] == "friends", "and the record says so")


def test_the_form_tells_the_truth_about_an_allowlist():
    print("test_the_form_tells_the_truth_about_an_allowlist")
    fresh_env()
    wizard = reimport()
    import identity
    good = identity.npub_encode("11" * 32)
    wizard.apply({"server_type": "survival", "server_name": "S", "access": "signin"})

    # The dashboard adds an allowlist after the wizard recorded "signin".
    identity.add_to_list("allow", good)
    form = wizard.current_state_for_form()
    check(form["access"] == "friends", "the form opens on 'Just my friends'")
    check(form["friends"] == [good], "with the list that is in force")

    # So an answer of signin or open really is what then applies.
    for answer, signin in (("signin", True), ("open", False)):
        identity.add_to_list("allow", good)
        wizard.apply({"server_type": "survival", "server_name": "S", "access": answer})
        check(not identity.allowlist_enabled(), f"{answer} turns the allowlist off")
        check(identity.require_signin() is signin, f"{answer} sets the sign-in rule")
        check(wizard.current_state_for_form()["access"] == answer, f"the form then shows {answer}")


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


def env_of(identity):
    parsed = {}
    for line in (identity.identity_dir() / "server.env").read_text().splitlines():
        if line and not line.startswith("#"):
            k, _, v = line.partition("=")
            parsed[k] = shlex.split(v)[0] if v else ""
    return parsed


def test_public_address_reaches_the_engine():
    print("test_public_address_reaches_the_engine")
    fresh_env()
    wizard = reimport()
    import identity
    wizard.apply({"server_type": "survival", "public_host": " play.example.org, 192.168.1.20:6767 "})
    check(env_of(identity).get("AXENSTAX_PUBLIC_HOST") == "play.example.org,192.168.1.20:6767",
          "the address lands in server.env")
    check(wizard.current_state_for_form()["public_host"] == "play.example.org,192.168.1.20:6767",
          "and the form shows it again")
    check(not wizard.restart_path().exists(), "a first run needs no restart")

    # "Add a world" and other runs that send no address keep the one it has.
    r = wizard.apply({"server_type": "survival"})
    check(env_of(identity).get("AXENSTAX_PUBLIC_HOST") == "play.example.org,192.168.1.20:6767",
          "a run without an address keeps it")
    check(r["restart"] is False, "and doesn't restart anything")

    r = wizard.apply({"server_type": "survival", "public_host": "other.example.org"})
    check(env_of(identity).get("AXENSTAX_PUBLIC_HOST") == "other.example.org", "a new address replaces it")
    check(r["restart"] is True, "and restarts the engine, which reads it at boot")


def test_a_bad_public_address_changes_nothing():
    print("test_a_bad_public_address_changes_nothing")
    fresh_env()
    wizard = reimport()
    import identity
    # What the engine would refuse at boot must be refused here instead.
    for bad in ("https://play.example.org", "play.example.org/ws", "user@host", "host:0",
                "host:70000", "a b", "[::1", "bad..name", "ex\u00e4mple.org", "host:port", "fe80::1%eth0"):
        raised = False
        try:
            wizard.apply({"server_type": "survival", "public_host": bad})
        except ValueError:
            raised = True
        check(raised, f"{bad!r} is refused")
    check(not (identity.identity_dir() / "server.env").exists(), "and nothing was written")
    check(not wizard.engine_booted_once(), "and the engine gate stays shut")
    for good in ("localhost", "192.168.1.20", "203.0.113.7:6767", "[2001:db8::1]:6767", "::1",
                 "play.example.org.", "my_box", "a.example,b.example:9000"):
        check(wizard.clean_public_host(good) == good, f"{good!r} is accepted")


def test_set_public_host_touches_only_the_address():
    print("test_set_public_host_touches_only_the_address")
    fresh_env()
    wizard = reimport()
    import identity
    wizard.apply({"server_type": "creative", "server_name": "Keep me"})
    before = env_of(identity)
    wizard.set_public_host("play.example.org")
    after = env_of(identity)
    check(after.pop("AXENSTAX_PUBLIC_HOST") == "play.example.org", "the address is set")
    check(after == before, "every other setting is as it was")
    check(wizard.restart_path().exists(), "and the engine is cycled to read it")
    raised = False
    try:
        wizard.set_public_host("")
    except ValueError:
        raised = True
    check(raised, "a blank address is refused")


def test_a_line_break_in_a_name_is_refused():
    print("test_a_line_break_in_a_name_is_refused")
    fresh_env()
    wizard = reimport()
    import identity
    for field in ("server_name", "about"):
        for value in ("a\nb", "a\rb"):
            try:
                wizard.apply({"server_type": "survival", field: value})
                check(False, f"{field} {value!r} is refused")
            except ValueError:
                check(True, f"{field} {value!r} is refused")
    check(not (identity.identity_dir() / "server.env").exists(), "and nothing was written")
    check(not (identity.identity_dir() / "server.json").exists(), "or recorded")
    # Spaces at the ends are only tidied, as they always were.
    wizard.apply({"server_type": "survival", "server_name": "  S  "})
    check(env_of(identity)["AXENSTAX_SERVER_NAME"] == "S", "an ordinary name is fine")


def test_a_damaged_env_file_does_not_stop_the_console():
    print("test_a_damaged_env_file_does_not_stop_the_console")
    fresh_env()
    wizard = reimport()
    import identity
    path = identity.identity_dir() / "server.env"
    path.parent.mkdir(parents=True, exist_ok=True)
    keep = b"AXENSTAX_GAMEMODE=creative\nAXENSTAX_PUBLIC_HOST=play.example.org\n"

    # What an older console could write: a value in quotes across two lines.
    path.write_bytes(b"AXENSTAX_SERVER_NAME='a\nb'\n" + keep)
    env = wizard._read_env()
    check(env.get("AXENSTAX_SERVER_NAME") == "a\nb", "a quoted value across two lines reads whole")
    check(env.get("AXENSTAX_GAMEMODE") == "creative", "with the lines after it")
    check("b'" not in env, "and its second line isn't a setting of its own")

    # Lines that can't be understood at all, among good ones.
    bad = b"\xff\xfe not text\nAXENSTAX_ABOUT='never closed\nnot a setting\nrm -rf x=y\n"
    path.write_bytes(b"# comment\n" + bad + keep)
    env = wizard._read_env()
    check(env == {"AXENSTAX_GAMEMODE": "creative", "AXENSTAX_PUBLIC_HOST": "play.example.org"},
          f"each bad line is skipped and the rest is kept: {env}")
    check(wizard.current_state_for_form()["public_host"] == "play.example.org", "the setup page opens")

    # The address route still works and the file it leaves can be sourced.
    wizard.set_public_host("new.example.org")
    env = wizard._read_env()
    check(env.get("AXENSTAX_PUBLIC_HOST") == "new.example.org", "the address can still be set")
    check(env.get("AXENSTAX_GAMEMODE") == "creative", "without losing the rest of the file")
    sourced = subprocess.run(["bash", "-c", f'set -e; . "{path}"; printf %s "$AXENSTAX_PUBLIC_HOST"'],
                             capture_output=True, text=True)
    check(sourced.returncode == 0 and sourced.stdout == "new.example.org",
          f"the entrypoint can source the result: {sourced.stderr}")
    wizard.apply({"server_type": "survival", "server_name": "S"})
    check(wizard._read_env().get("AXENSTAX_PUBLIC_HOST") == "new.example.org", "and the wizard can run")

    # Other line separators in a value don't split it.
    wizard.set_public_host("new.example.org")
    wizard._write_env({"AXENSTAX_SERVER_NAME": "a\u2028b", "AXENSTAX_GAMEMODE": "creative"})
    env = wizard._read_env()
    check(env.get("AXENSTAX_SERVER_NAME") == "a\u2028b", "a Unicode line separator stays in its value")

    # A file that isn't a file reads as empty, as a missing one does.
    path.unlink()
    path.mkdir()
    check(wizard._read_env() == {}, "an unreadable file reads as empty")


def run_entrypoint(worlds, compose_host=""):
    """Run the real entrypoint with stand-ins for Caddy and the engine: the engine
    records the address it was started with, and both just sit there. Exported shell
    functions, not scripts: a temp dir may well be mounted noexec."""
    import subprocess
    started = Path(tempfile.mkdtemp(prefix="wiz-entry-")) / "engine-started"
    env = {"PATH": os.environ["PATH"], "AXENSTAX_WORLDS_DIR": str(worlds), "AXENSTAX_SKIP_WIZARD": "1",
           "BASH_FUNC_caddy%%": "() { exec sleep 60; }",
           "BASH_FUNC_openssl%%": "() { :; }",
           "BASH_FUNC_axenstax-engine%%":
               f'() {{ echo "$AXENSTAX_PUBLIC_HOST" > "{started}"; exec sleep 60; }}'}
    if compose_host:
        env["AXENSTAX_PUBLIC_HOST"] = compose_host
    entrypoint = Path(__file__).parents[2] / "dedicated-server" / "entrypoint.sh"
    proc = subprocess.Popen(["bash", str(entrypoint)], env=env, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL)
    return proc, started


def stop_entrypoint(proc):
    proc.terminate()
    proc.wait(timeout=15)


def wait_for(path, seconds=10):
    import time
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if path.exists():
            return True
        time.sleep(0.1)
    return False


def read_when_written(path, seconds=5):
    """The stand-in creates the file, then writes it."""
    import time
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline and not path.read_text().strip():
        time.sleep(0.05)
    return path.read_text().strip()


def test_the_entrypoint_waits_for_an_address_instead_of_looping():
    print("test_the_entrypoint_waits_for_an_address_instead_of_looping")
    worlds = fresh_env()
    wizard = reimport()
    proc, started = run_entrypoint(worlds)
    try:
        marker = wizard.awaiting_public_host_path()
        check(wait_for(marker), "with no address it waits, and says so to the console")
        check(wizard.awaiting_public_host(), "the console can tell")
        check(not started.exists(), "the engine isn't started to refuse and bring the box down")
        check(proc.poll() is None, "the box stays up so the console can be reached")
        wizard.set_public_host("play.example.org")
        check(wait_for(started), "an address from the console releases it, with no restart")
        check(read_when_written(started) == "play.example.org", "the engine gets that address")
        check(not marker.exists(), "and the console stops asking")
    finally:
        stop_entrypoint(proc)


def test_the_entrypoint_keeps_an_address_from_compose():
    print("test_the_entrypoint_keeps_an_address_from_compose")
    worlds = fresh_env()
    wizard = reimport()
    wizard.set_public_host("from.wizard")
    proc, started = run_entrypoint(worlds, compose_host="from.compose")
    try:
        check(wait_for(started), "an address named in compose needs no waiting")
        check(read_when_written(started) == "from.compose,from.wizard",
              "and the wizard's adds to it rather than dropping it")
    finally:
        stop_entrypoint(proc)


def main():
    # Make local imports resolve when run from anywhere.
    sys.path.insert(0, str(Path(__file__).parent))
    for t in (test_first_run, test_apply_gallery, test_gallery_open_toggle, test_env_quoting,
              test_signin_is_the_default, test_a_run_with_no_access_answer_changes_no_access,
              test_the_form_tells_the_truth_about_an_allowlist,
              test_friends, test_rerun_mode_change_resets_world,
              test_rerun_same_mode_no_reset,
              test_apply_terrain_flat, test_rerun_terrain_change_resets_world,
              test_invalid_terrain_falls_back_to_defaults, test_skip,
              test_public_address_reaches_the_engine, test_a_bad_public_address_changes_nothing,
              test_set_public_host_touches_only_the_address,
              test_a_line_break_in_a_name_is_refused,
              test_a_damaged_env_file_does_not_stop_the_console,
              test_the_entrypoint_waits_for_an_address_instead_of_looping,
              test_the_entrypoint_keeps_an_address_from_compose):
        t()
    print()
    if _fails:
        print(f"{_fails} check(s) FAILED")
        sys.exit(1)
    print("all wizard checks passed")


if __name__ == "__main__":
    main()
