#!/usr/bin/env python3
"""Capability-scoped trusted-signer set (`identity.pubkeys_with_cap`).

    python3 test_identity_caps.py

Guards the publish-authz gate: the game-signed publish path must trust only
holders of the `settings` capability (owner/admin), NOT every operator — a
moderator holds a role (so it can sign in) but only the `access` cap, and must
not be able to authorize a content publish the web console reserves for
owner/admin. Runs against a throwaway identity dir, no crypto needed.
"""

import os
import sys
import tempfile
from pathlib import Path

_fails = 0

# Distinct 64-hex pubkeys per role — role files accept raw hex directly.
OWNER = "11" * 32
ADMIN = "22" * 32
MOD = "33" * 32


def check(cond, msg):
    global _fails
    if cond:
        print(f"  ok  — {msg}")
    else:
        _fails += 1
        print(f" FAIL — {msg}")


def fresh_env():
    tmp = tempfile.mkdtemp(prefix="idcaps-test-")
    os.environ["AXENSTAX_IDENTITY_DIR"] = str(Path(tmp) / ".identity")
    os.environ.pop("AXENSTAX_WORLDS_DIR", None)
    sys.modules.pop("identity", None)
    import identity
    d = identity.identity_dir()
    d.mkdir(parents=True, exist_ok=True)
    (d / "owners.txt").write_text(OWNER + "\n", encoding="utf-8")
    (d / "admins.txt").write_text(ADMIN + "\n", encoding="utf-8")
    (d / "moderators.txt").write_text(MOD + "\n", encoding="utf-8")
    return identity


def test_settings_signer_set_excludes_moderator():
    print("test_settings_signer_set_excludes_moderator")
    identity = fresh_env()

    # Sanity: all three roles can sign in (the old, too-broad gate).
    signin = {p.lower() for p in identity.operator_pubkeys_hex()}
    check({OWNER, ADMIN, MOD} <= signin, "owner+admin+moderator all in sign-in set")

    # The publish gate: `settings`-capable keys only — owner + admin, NOT moderator.
    publishers = {p.lower() for p in identity.pubkeys_with_cap("settings")}
    check(OWNER in publishers, "owner is a settings-capable signer")
    check(ADMIN in publishers, "admin is a settings-capable signer")
    check(MOD not in publishers,
          "moderator EXCLUDED from settings signer set (the bypass fix)")

    # The moderator's own cap (`access`) still resolves — it's just scoped down.
    access = {p.lower() for p in identity.pubkeys_with_cap("access")}
    check({OWNER, ADMIN, MOD} <= access, "all roles hold `access`")
    check(identity.pubkeys_with_cap("no-such-cap") == set(),
          "unknown cap yields an empty signer set (deny by default)")


def main():
    sys.path.insert(0, str(Path(__file__).parent))
    test_settings_signer_set_excludes_moderator()
    print()
    if _fails:
        print(f"{_fails} check(s) FAILED")
        sys.exit(1)
    print("all identity-cap checks passed")


if __name__ == "__main__":
    main()
