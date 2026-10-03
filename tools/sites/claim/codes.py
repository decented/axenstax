"""Single-use claim codes — the only thing standing between the public form and
your Printful bill.

Each code authorises exactly ONE order. You generate a batch (gen_codes.py),
hand them out after taking payment / as prizes, and the buyer redeems one to ship
their shirt. Codes are reserved atomically so a leaked or shared code can't create
two orders, and a failed order releases its code again.

Stored as JSON on disk (no DB). Structure:
  { "ABCD2345": {"created_at": 1700000000, "used_at": null, "order_id": null}, ... }
"""

import json
import os
import secrets
import threading
import time
from pathlib import Path

# Unambiguous alphabet — no 0/O/1/I to avoid read-aloud / typo confusion.
_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"
_LOCK = threading.RLock()


def _data_dir() -> Path:
    return Path(os.environ.get("CLAIM_DATA_DIR", Path(__file__).parent / "data"))


def _codes_file() -> Path:
    return _data_dir() / "codes.json"


def _load() -> dict:
    f = _codes_file()
    if not f.exists():
        return {}
    try:
        return json.loads(f.read_text() or "{}")
    except json.JSONDecodeError:
        return {}


def _save(data: dict) -> None:
    d = _data_dir()
    d.mkdir(parents=True, exist_ok=True)
    tmp = _codes_file().with_suffix(".tmp")
    tmp.write_text(json.dumps(data, indent=2, sort_keys=True))
    tmp.replace(_codes_file())  # atomic on POSIX


def normalise(code: str) -> str:
    return (code or "").strip().upper().replace("-", "").replace(" ", "")


def status(code: str) -> str:
    """'valid' | 'used' | 'unknown' — for the GET-time gate (does NOT consume)."""
    code = normalise(code)
    if not code:
        return "unknown"
    with _LOCK:
        rec = _load().get(code)
    if rec is None:
        return "unknown"
    return "used" if rec.get("used_at") else "valid"


def reserve(code: str) -> bool:
    """Atomically mark a valid+unused code as used. Returns True if it was
    reservable (now reserved), False otherwise. Call this BEFORE creating the
    Printful order, then set_order() on success or release() on failure."""
    code = normalise(code)
    if not code:
        return False
    with _LOCK:
        data = _load()
        rec = data.get(code)
        if rec is None or rec.get("used_at"):
            return False
        rec["used_at"] = int(time.time())
        _save(data)
        return True


def set_order(code: str, order_id) -> None:
    code = normalise(code)
    with _LOCK:
        data = _load()
        if code in data:
            data[code]["order_id"] = order_id
            _save(data)


def release(code: str) -> None:
    """Undo a reservation (order creation failed), so the code can be reused."""
    code = normalise(code)
    with _LOCK:
        data = _load()
        rec = data.get(code)
        if rec is not None:
            rec["used_at"] = None
            rec["order_id"] = None
            _save(data)


def generate(n: int, length: int = 8) -> list[str]:
    """Create n new unique codes, append them to the store, return them."""
    new: dict = {}
    with _LOCK:
        data = _load()
        while len(new) < n:
            code = "".join(secrets.choice(_ALPHABET) for _ in range(length))
            if code in data or code in new:
                continue
            new[code] = {"created_at": int(time.time()), "used_at": None, "order_id": None}
        data.update(new)
        _save(data)
    return list(new.keys())


def summary() -> dict:
    with _LOCK:
        data = _load()
    used = sum(1 for r in data.values() if r.get("used_at"))
    return {"total": len(data), "used": used, "available": len(data) - used}
