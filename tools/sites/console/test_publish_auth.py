#!/usr/bin/env python3
"""Tests for the operator-signed publish-authorization verifier (build spec §3).

    ../game/.venv/bin/python test_publish_auth.py

Needs `secp256k1` (to SIGN test events) + `fastapi` (auth.py imports it), so run
with one of the site venvs, not bare system python. Signs via the same low-level
libsecp256k1 binding auth.py verifies against, so a green run proves the real
sign↔verify path end-to-end (no mocking of the crypto)."""

import hashlib
import json
import sys
import time
from pathlib import Path

from secp256k1._libsecp256k1 import ffi as _ffi, lib as _lib

_ctx = _lib.secp256k1_context_create(0x0301)  # SIGN | VERIFY

_fails = 0


def check(cond, msg):
    global _fails
    if cond:
        print(f"  ok  — {msg}")
    else:
        _fails += 1
        print(f" FAIL — {msg}")


def _keypair(seed: bytes):
    """Return (keypair_cdata, xonly_pubkey_hex) for a 32-byte secret."""
    kp = _ffi.new("secp256k1_keypair *")
    if _lib.secp256k1_keypair_create(_ctx, kp, seed) != 1:
        raise RuntimeError("invalid secret key")
    xonly = _ffi.new("secp256k1_xonly_pubkey *")
    _lib.secp256k1_keypair_xonly_pub(_ctx, xonly, _ffi.NULL, kp)
    out = _ffi.new("unsigned char[32]")
    _lib.secp256k1_xonly_pubkey_serialize(_ctx, out, xonly)
    return kp, bytes(_ffi.buffer(out, 32)).hex()


def _event_id(pubkey, created_at, kind, tags, content):
    payload = json.dumps([0, pubkey, created_at, kind, tags, content],
                         separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def _sign(kp, msg32: bytes) -> str:
    sig = _ffi.new("unsigned char[64]")
    aux = bytes(32)
    if _lib.secp256k1_schnorrsig_sign(_ctx, sig, msg32, kp, aux) != 1:
        raise RuntimeError("sign failed")
    return bytes(_ffi.buffer(sig, 64)).hex()


def make_event(kp, pubkey_hex, *, kind, tags, content="axenstax-publish", created_at=None):
    if created_at is None:
        created_at = int(time.time())
    eid = _event_id(pubkey_hex, created_at, kind, tags, content)
    sig = _sign(kp, bytes.fromhex(eid))
    return {"id": eid, "pubkey": pubkey_hex, "created_at": created_at,
            "kind": kind, "tags": tags, "content": content, "sig": sig}


def main():
    sys.path.insert(0, str(Path(__file__).parent))
    import auth

    K = auth.PUBLISH_AUTHZ_KIND
    kp, pub = _keypair(b"\x01" * 32)
    _kp2, pub2 = _keypair(b"\x02" * 32)  # a non-operator
    operators = {pub}
    world = "server-world"
    sha = "ab" * 32  # 64-hex archive hash
    tags = [["world", world], ["x", sha], ["nonce", "deadbeef"]]

    # 1. Happy path.
    ev = make_event(kp, pub, kind=K, tags=tags)
    op, err = auth.verify_publish_authorization(ev, world, sha, operators)
    check(err is None and op == pub, "valid operator-signed publish authz accepted")

    # 2. Foreign (non-operator) signer.
    ev2 = make_event(_kp2, pub2, kind=K, tags=tags)
    _op, err = auth.verify_publish_authorization(ev2, world, sha, operators)
    check(err == "not-operator", "non-operator signer rejected")

    # 3. Archive-hash mismatch (anti-swap): a different uploaded blob.
    _op, err = auth.verify_publish_authorization(ev, world, "cd" * 32, operators)
    check(err == "hash-mismatch", "archive-hash mismatch rejected (anti-swap)")

    # 4. World mismatch (no cross-world publish).
    _op, err = auth.verify_publish_authorization(ev, "other-world", sha, operators)
    check(err == "world-mismatch", "world-id mismatch rejected")

    # 5. Tampered content — recomputed id won't match the signed id.
    bad = dict(ev)
    bad["content"] = "tampered"
    _op, err = auth.verify_publish_authorization(bad, world, sha, operators)
    check(err == "invalid-event-id", "tampered content breaks the event id")

    # 6. Tampered signature.
    bad = dict(ev)
    bad["sig"] = "00" * 64
    _op, err = auth.verify_publish_authorization(bad, world, sha, operators)
    check(err == "invalid-signature", "forged signature rejected")

    # 7. Stale event (older than the window).
    old = make_event(kp, pub, kind=K, tags=tags,
                     created_at=int(time.time()) - auth.PUBLISH_AUTHZ_SKEW - 60)
    _op, err = auth.verify_publish_authorization(old, world, sha, operators)
    check(err == "too-old", "stale authz rejected (anti-replay)")

    # 8. Future event.
    fut = make_event(kp, pub, kind=K, tags=tags,
                     created_at=int(time.time()) + auth.PUBLISH_AUTHZ_SKEW + 60)
    _op, err = auth.verify_publish_authorization(fut, world, sha, operators)
    check(err == "in-the-future", "future-dated authz rejected")

    # 9. Wrong kind.
    wrong = make_event(kp, pub, kind=1, tags=tags)
    _op, err = auth.verify_publish_authorization(wrong, world, sha, operators)
    check(err == "wrong-kind", "wrong event kind rejected")

    # 10. Case-insensitive hash compare (the game may send upper/lower hex).
    upper_tags = [["world", world], ["x", sha.upper()], ["nonce", "f00d"]]
    ev_up = make_event(kp, pub, kind=K, tags=upper_tags)
    op, err = auth.verify_publish_authorization(ev_up, world, sha.lower(), operators)
    check(err is None and op == pub, "hash compare is case-insensitive")

    print()
    if _fails:
        print(f"{_fails} check(s) FAILED")
        sys.exit(1)
    print("all publish-auth checks passed")


if __name__ == "__main__":
    main()
