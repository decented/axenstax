#!/usr/bin/env python3
"""Stdlib tests for the console's access-policy files (no pytest needed):

    python3 test_access_policy.py

The engine reloads these files every few seconds, so what the console leaves on
disk IS the policy: an absent whitelist.txt means no allowlist, an empty one means
nobody may join, and a file the console could not read is never rewritten.
"""

import asyncio
import os
import stat
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


def main():
    sys.path.insert(0, str(Path(__file__).parent))
    for t in (test_last_removal_keeps_the_allowlist_on,
              test_removal_never_turns_an_absent_allowlist_on,
              test_read_failure_writes_nothing, test_writes_replace_the_file_whole,
              test_allowlist_off_deletes_the_file,
              test_a_failed_allowlist_off_is_a_server_error,
              test_the_wizard_moves_the_allowlist_only_when_asked,
              test_require_signin_reads_as_the_engine_would_refuse):
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
