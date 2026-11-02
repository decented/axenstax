#!/usr/bin/env python3
"""Stdlib tests for the console's access-policy files (no pytest needed):

    python3 test_access_policy.py

The engine reloads these files every few seconds, so what the console leaves on
disk IS the policy: an absent whitelist.txt means no allowlist, an empty one means
nobody may join, and a file the console could not read is never rewritten.
"""

import asyncio
import fcntl
import os
import stat
import sys
import tempfile
import threading
import time
from pathlib import Path

_fails = 0



def check(cond, msg):
    global _fails
    if cond:
        print(f"  ok  — {msg}")
    else:
        _fails += 1
        print(f" FAIL — {msg}")


def fresh():
    """A throwaway identity dir, and the module re-imported against it."""
    tmp = tempfile.mkdtemp(prefix="policy-test-")
    os.environ["AXENSTAX_WORLDS_DIR"] = str(Path(tmp) / "worlds")
    os.environ.pop("AXENSTAX_IDENTITY_DIR", None)
    os.environ["AXENSTAX_WORLD"] = "server-world"
    for m in ("wizard", "studio", "identity"):
        sys.modules.pop(m, None)
    import identity
    identity.identity_dir().mkdir(parents=True)
    return identity


def valid_npubs(identity):
    """Two npubs this module accepts (derived, so the test holds no hand-made bech32)."""
    return identity.npub_encode("11" * 32), identity.npub_encode("22" * 32)


def test_last_removal_keeps_the_allowlist_on():
    print("test_last_removal_keeps_the_allowlist_on")
    identity = fresh()
    a, _ = valid_npubs(identity)
    identity.add_to_list("allow", a)
    identity.remove_from_list("allow", a)
    check((identity.identity_dir() / "whitelist.txt").exists(), "the emptied file stays")
    check(identity.allowlist_enabled(), "so the allowlist is still on: nobody may join")


def test_removal_never_turns_an_absent_allowlist_on():
    print("test_removal_never_turns_an_absent_allowlist_on")
    identity = fresh()
    a, _ = valid_npubs(identity)
    identity.remove_from_list("allow", a)
    check(not (identity.identity_dir() / "whitelist.txt").exists(),
          "removing from no allowlist does not create one and close the server")


def test_read_failure_writes_nothing():
    print("test_read_failure_writes_nothing")
    identity = fresh()
    a, b = valid_npubs(identity)
    identity.add_to_list("allow", a)
    identity.add_to_list("allow", b)
    path = identity.identity_dir() / "whitelist.txt"
    before = path.read_bytes()
    path.chmod(stat.S_IWUSR)
    try:
        path.read_bytes()
        print("  (running as root: nothing to show)")
        return
    except OSError:
        pass
    raised = False
    try:
        identity.remove_from_list("allow", a)
    except ValueError:
        raised = True
    path.chmod(stat.S_IRUSR | stat.S_IWUSR)
    check(raised, "the failed read is reported")
    check(path.read_bytes() == before, "and the file is untouched")


def test_writes_replace_the_file_whole():
    print("test_writes_replace_the_file_whole")
    identity = fresh()
    a, b = valid_npubs(identity)
    identity.add_to_list("block", a)
    path = identity.identity_dir() / "blocklist.txt"
    inode = path.stat().st_ino
    identity.add_to_list("block", b)
    check(path.stat().st_ino != inode, "a new file is renamed into place, never truncated")
    check(identity.blocklist() == [a, b], "with both entries")
    leftovers = [p.name for p in identity.identity_dir().iterdir() if p.name.endswith(".tmp")]
    check(not leftovers, f"no temp file left behind: {leftovers}")
    mode = stat.S_IMODE(path.stat().st_mode)
    check(mode & stat.S_IROTH, "readable by the engine, which runs in another container")


def test_allowlist_off_deletes_the_file():
    print("test_allowlist_off_deletes_the_file")
    identity = fresh()
    a, _ = valid_npubs(identity)
    identity.add_to_list("allow", a)
    identity.allowlist_off()
    check(not identity.allowlist_enabled(), "the allowlist is off")
    check(not (identity.identity_dir() / "whitelist.txt").exists(), "the file is gone")
    identity.allowlist_off()
    check(not identity.allowlist_enabled(), "turning it off twice is fine")


def test_require_signin_reads_as_the_engine_would_refuse():
    print("test_require_signin_reads_as_the_engine_would_refuse")
    identity = fresh()
    path = identity.identity_dir() / "require_signin"
    path.write_text("")
    check(identity.require_signin() is True, "an empty file is not reported as guests admitted")
    path.write_text("  FALSE\n")
    check(identity.require_signin() is False, "'false' is trimmed and any case")
    path.write_text("TRUE")
    check(identity.require_signin() is True, "'true' likewise")


def test_the_wizard_moves_the_allowlist_only_when_asked():
    print("test_the_wizard_moves_the_allowlist_only_when_asked")
    identity = fresh()
    import wizard
    wizard.apply({"server_type": "survival", "access": "friends", "friends": ""})
    check(identity.allowlist_enabled(), "'just my friends' with nobody named admits nobody")
    wizard.apply({"server_type": "survival"})
    check(identity.allowlist_enabled(), "a run that sends no access leaves it alone")
    wizard.apply({"server_type": "survival", "access": "friends", "friends": ""})
    wizard.apply({"server_type": "survival", "access": "signin"})
    check(not identity.allowlist_enabled(), "moving off 'friends' turns it off")


def test_a_failed_allowlist_off_is_a_server_error():
    print("test_a_failed_allowlist_off_is_a_server_error")
    fresh()
    import app
    from fastapi import HTTPException

    def refuse():
        raise OSError("read-only file system")

    app.identity.allowlist_off = refuse
    app._require_cap = lambda request, cap: b"operator"
    try:
        asyncio.run(app.allowlist_off(None, x_requested_with="fetch"))
        check(False, "the failure is reported")
    except HTTPException as e:
        check(e.status_code == 500, f"it is a 500, not the caller's fault: {e.status_code}")


def test_the_dashboard_tells_an_open_server_from_a_closed_one():
    print("test_the_dashboard_tells_an_open_server_from_a_closed_one")
    from jinja2 import Template
    identity = fresh()
    a, _ = valid_npubs(identity)
    html = (Path(__file__).parent / "templates" / "dashboard.html").read_text(encoding="utf-8")
    start = html.index("<h3>Allowlist")
    section = Template(html[start:html.index("<h3>Blocklist", start)])

    def shown():
        return section.render(allowlist=identity.allowlist(), allowlist_on=identity.allowlist_enabled())

    open_page = shown()
    check("Off: anyone the sign-in rule admits may join" in open_page, "no whitelist.txt reads as Off, open")
    check("nobody can join" not in open_page, "and does not say nobody can join")
    identity.turn_allowlist_on()
    closed_page = shown()
    check("On: only the npubs below may join" in closed_page, "an empty whitelist.txt reads as On")
    check("empty \u2014 nobody can join" in closed_page, "and says that nobody can join")
    identity.add_to_list("allow", a)
    check(a in shown(), "a listed npub is shown")


def test_a_write_keeps_the_mode_of_the_file_it_replaces():
    print("test_a_write_keeps_the_mode_of_the_file_it_replaces")
    identity = fresh()
    a, b = valid_npubs(identity)
    path = identity.identity_dir() / "whitelist.txt"
    identity.add_to_list("allow", a)
    path.chmod(0o600)
    identity.add_to_list("allow", b)
    check(stat.S_IMODE(path.stat().st_mode) == 0o600, "a hand-tightened 0600 allowlist stays 0600")
    identity.remove_from_list("allow", a)
    check(stat.S_IMODE(path.stat().st_mode) == 0o600, "through a removal too")
    check(identity.allowlist() == [b], "and the content is right")


def test_writers_do_not_share_a_dot_tmp_name():
    print("test_writers_do_not_share_a_dot_tmp_name")
    identity = fresh()
    a, _ = valid_npubs(identity)
    theirs = identity.identity_dir() / "whitelist.txt.tmp"
    theirs.write_text("the engine's half-written list", encoding="utf-8")
    identity.add_to_list("allow", a)
    check(theirs.read_text(encoding="utf-8") == "the engine's half-written list",
          "another writer's <file>.tmp is left alone")
    check(identity.allowlist() == [a], "and the list is written")
    leftovers = [p.name for p in identity.identity_dir().iterdir()
                 if p.name.endswith(".tmp") and p != theirs]
    check(not leftovers, f"our own temp file is renamed away: {leftovers}")


def test_a_change_waits_for_the_policy_lock():
    print("test_a_change_waits_for_the_policy_lock")
    identity = fresh()
    a, b = valid_npubs(identity)
    lock_path = identity.identity_dir() / ".policy.lock"
    # What the engine does around its own read-modify-write: flock this file.
    held = open(lock_path, "a")
    fcntl.flock(held, fcntl.LOCK_EX)
    worker = threading.Thread(target=lambda: identity.add_to_list("allow", a))
    worker.start()
    time.sleep(0.3)
    check(not (identity.identity_dir() / "whitelist.txt").exists(),
          "nothing is written while the engine holds the lock")
    fcntl.flock(held, fcntl.LOCK_UN)
    held.close()
    worker.join(5)
    check(not worker.is_alive() and identity.allowlist() == [a], "it goes ahead once the lock is free")

    # And two console writers in a row lose nothing.
    threads = [threading.Thread(target=identity.queue_kick, args=(n,))
               for n in (a, b, a, b, a, b)]
    for t in threads:
        t.start()
    for t in threads:
        t.join(5)
    queued = (identity.identity_dir() / "kick").read_text(encoding="utf-8").split()
    check(len(queued) == 6, f"six kicks queued at once are all there: {len(queued)}")


def main():
    sys.path.insert(0, str(Path(__file__).parent))
    for t in (test_last_removal_keeps_the_allowlist_on,
              test_removal_never_turns_an_absent_allowlist_on,
              test_read_failure_writes_nothing, test_writes_replace_the_file_whole,
              test_allowlist_off_deletes_the_file,
              test_a_failed_allowlist_off_is_a_server_error,
              test_the_wizard_moves_the_allowlist_only_when_asked,
              test_require_signin_reads_as_the_engine_would_refuse,
              test_the_dashboard_tells_an_open_server_from_a_closed_one,
              test_a_write_keeps_the_mode_of_the_file_it_replaces,
              test_writers_do_not_share_a_dot_tmp_name,
              test_a_change_waits_for_the_policy_lock):
        try:
            t()
        except Exception as e:  # a missing function is a failure, not a crash of the run
            global _fails
            _fails += 1
            print(f" FAIL — {t.__name__}: {type(e).__name__}: {e}")
    print()
    if _fails:
        print(f"{_fails} check(s) FAILED")
        sys.exit(1)
    print("all access-policy checks passed")


if __name__ == "__main__":
    main()
