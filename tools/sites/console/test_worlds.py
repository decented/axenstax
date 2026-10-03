#!/usr/bin/env python3
"""Stdlib tests for the served-world manifest (no pytest needed):

    python3 test_worlds.py

Runs against a throwaway identity dir so it never touches a real volume.
"""

import os
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
    tmp = tempfile.mkdtemp(prefix="worlds-test-")
    os.environ["AXENSTAX_WORLDS_DIR"] = str(Path(tmp) / "worlds")
    os.environ.pop("AXENSTAX_IDENTITY_DIR", None)
    for m in ("identity", "worlds"):
        sys.modules.pop(m, None)
    import worlds  # noqa: F401
    return worlds


def test_empty_manifest_is_safe():
    print("test_empty_manifest_is_safe")
    worlds = fresh_env()
    check(worlds.list_worlds() == [], "missing manifest reads as empty list")
    check(worlds.get_world("nope") is None, "get_world on empty ⇒ None")
    check(worlds.remove_world("nope") is False, "remove on empty ⇒ False")


def test_record_and_get():
    print("test_record_and_get")
    worlds = fresh_env()
    stored = worlds.record_world({
        "id": "server-world",
        "title": "My Gallery",
        "kind": "gallery",
        "access": "open",
        "content_owner_npub": "npub1owner",
        "world_type": "flat",
        "ground": "sand",
        "water_depth": 3,
        "game_mode": "creative",
    })
    check(stored["created_unix"] > 0, "created_unix stamped")
    check(stored["updated_unix"] >= stored["created_unix"], "updated_unix stamped")
    got = worlds.get_world("server-world")
    check(got is not None and got["title"] == "My Gallery", "get_world returns the entry")
    check(len(worlds.list_worlds()) == 1, "one world listed")


def test_upsert_preserves_created_unix():
    print("test_upsert_preserves_created_unix")
    worlds = fresh_env()
    first = worlds.record_world({"id": "w", "title": "v1", "kind": "normal"})
    born = first["created_unix"]
    # A re-publish overwrites content but keeps the world's birth date + count.
    second = worlds.record_world({"id": "w", "title": "v2", "kind": "gallery"})
    check(second["created_unix"] == born, "created_unix preserved across re-publish")
    check(second["title"] == "v2", "title updated")
    check(second["kind"] == "gallery", "kind updated")
    check(len(worlds.list_worlds()) == 1, "upsert, not append — still one world")


def test_record_requires_id():
    print("test_record_requires_id")
    worlds = fresh_env()
    try:
        worlds.record_world({"title": "no id"})
        check(False, "record without id should raise")
    except ValueError:
        check(True, "record without id raises ValueError")


def test_remove():
    print("test_remove")
    worlds = fresh_env()
    worlds.record_world({"id": "a", "title": "A"})
    worlds.record_world({"id": "b", "title": "B"})
    check(worlds.remove_world("a") is True, "remove existing ⇒ True")
    ids = [w["id"] for w in worlds.list_worlds()]
    check(ids == ["b"], "only b remains")


def test_malformed_manifest_is_tolerated():
    print("test_malformed_manifest_is_tolerated")
    worlds = fresh_env()
    p = worlds.manifest_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text("{ not json at all", encoding="utf-8")
    check(worlds.list_worlds() == [], "garbage manifest reads as empty (never raises)")
    # And a write still works afterwards (overwrites the garbage).
    worlds.record_world({"id": "x", "title": "X"})
    check(len(worlds.list_worlds()) == 1, "record recovers from a garbage manifest")


def test_pack_world_dir():
    print("test_pack_world_dir")
    worlds = fresh_env()
    import io
    import tarfile
    base = worlds.world_slot_dir("server-world")
    (base / "chunks").mkdir(parents=True, exist_ok=True)
    (base / "exhibits").mkdir(parents=True, exist_ok=True)
    (base / "world_meta.json").write_text('{"display_name":"G"}', encoding="utf-8")
    (base / "world.dat").write_bytes(b"\x00\x01\x02 world data")
    (base / "chunks" / "0_0_0.chunk").write_bytes(b"chunkbytes")
    (base / "exhibits" / "art.png").write_bytes(b"\x89PNG imagebytes")
    blob = worlds.pack_world_dir("server-world")
    check(blob is not None and len(blob) > 0, "pack produced bytes")
    names = set()
    with tarfile.open(fileobj=io.BytesIO(blob), mode="r:gz") as tf:
        for m in tf.getmembers():
            names.add(m.name)
    check("world_meta.json" in names, "meta member present")
    check("world.dat" in names, "world.dat member present")
    check("chunks/0_0_0.chunk" in names, "chunk member present")
    check("exhibits/art.png" in names, "exhibit image member present (gallery fix)")


def test_pack_missing_or_unsafe_world_is_none():
    print("test_pack_missing_or_unsafe_world_is_none")
    worlds = fresh_env()
    check(worlds.pack_world_dir("nonexistent") is None, "missing world ⇒ None")
    check(worlds.pack_world_dir("../escape") is None, "traversal id ⇒ None (rejected)")
    check(worlds.is_valid_world_id("server-world") is True, "plain id is valid")
    check(worlds.is_valid_world_id("../x") is False, "traversal id is invalid")


def test_install_published_world_round_trip():
    print("test_install_published_world_round_trip")
    worlds = fresh_env()
    src = worlds.world_slot_dir("src-world")
    (src / "chunks").mkdir(parents=True, exist_ok=True)
    (src / "exhibits").mkdir(parents=True, exist_ok=True)
    (src / "world_meta.json").write_text('{"display_name":"Src"}', encoding="utf-8")
    (src / "world.dat").write_bytes(b"WORLDDATA")
    (src / "chunks" / "1_2_3.chunk").write_bytes(b"chunk")
    # Real PNG magic bytes — the extractor now sniffs exhibits (2026-09-27 audit
    # fix), so a fixture that merely spelled "PNG" as ASCII would get dropped.
    png_bytes = b"\x89PNG\r\n\x1a\n" + b"fakepngbody"
    (src / "exhibits" / "art.png").write_bytes(png_bytes)
    blob = worlds.pack_world_dir("src-world")
    # Publish (install) into a DIFFERENT slot — the full pack→install round trip.
    ok, err = worlds.install_published_world("dst-world", blob)
    check(ok and err is None, "install succeeds")
    dst = worlds.world_slot_dir("dst-world")
    import json as _json
    dst_meta = _json.loads((dst / "world_meta.json").read_text())
    check(dst_meta.get("display_name") == "Src", "meta installed")
    check((dst / "world.dat").read_bytes() == b"WORLDDATA", "world.dat installed")
    check((dst / "chunks" / "1_2_3.chunk").read_bytes() == b"chunk", "chunk installed")
    check((dst / "exhibits" / "art.png").read_bytes() == png_bytes, "exhibit image installed")
    # Re-publish (slot already exists) must overwrite cleanly via the atomic swap.
    ok2, _ = worlds.install_published_world("dst-world", blob)
    check(ok2, "re-publish over an existing slot succeeds (atomic swap)")


def test_pop_secret_stripped_on_export_and_fresh_on_import():
    print("test_pop_secret_stripped_on_export_and_fresh_on_import")
    worlds = fresh_env()
    import io
    import json as _json
    import tarfile
    src = worlds.world_slot_dir("sec-world")
    src.mkdir(parents=True, exist_ok=True)
    old_secret = [7] * 32
    (src / "world_meta.json").write_text(
        _json.dumps({"display_name": "S", "pop_secret": old_secret}), encoding="utf-8")
    (src / "world.dat").write_bytes(b"WORLDDATA")
    blob = worlds.pack_world_dir("sec-world")
    with tarfile.open(fileobj=io.BytesIO(blob), mode="r:gz") as tf:
        exported = _json.loads(tf.extractfile("world_meta.json").read())
    check("pop_secret" not in exported, "export strips the PoP secret")
    check(exported.get("display_name") == "S", "export keeps the rest of the meta")
    # Import a hand-built archive that still CARRIES a secret: it must be replaced.
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tf:
        for name, data in (
            ("world_meta.json", _json.dumps({"display_name": "S", "pop_secret": old_secret}).encode()),
            ("world.dat", b"WORLDDATA"),
        ):
            ti = tarfile.TarInfo(name)
            ti.size = len(data)
            tf.addfile(ti, io.BytesIO(data))
    ok, err = worlds.install_published_world("sec-dst", buf.getvalue())
    check(ok and err is None, "install with a secret succeeds")
    got = _json.loads((worlds.world_slot_dir("sec-dst") / "world_meta.json").read_text())
    fresh = got.get("pop_secret")
    check(isinstance(fresh, list) and len(fresh) == 32, "import gets a fresh 32-byte secret")
    check(fresh != old_secret, "import does not keep the sender's secret")


def test_install_rejects_incomplete_and_unsafe():
    print("test_install_rejects_incomplete_and_unsafe")
    worlds = fresh_env()
    import io
    import tarfile
    # Archive missing world.dat → rejected (not a complete world).
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tf:
        data = b"{}"
        info = tarfile.TarInfo("world_meta.json")
        info.size = len(data)
        tf.addfile(info, io.BytesIO(data))
    ok, err = worlds.install_published_world("x", buf.getvalue())
    check(not ok and "incomplete" in (err or ""), "archive without world.dat rejected")
    # Garbage bytes → rejected, not crashed.
    ok, err = worlds.install_published_world("x", b"not a tar at all")
    check(not ok, "garbage archive rejected")
    # A traversal member is dropped, leaving the archive incomplete → rejected.
    buf2 = io.BytesIO()
    with tarfile.open(fileobj=buf2, mode="w:gz") as tf:
        for name, data in (("world.dat", b"d"), ("../evil", b"x")):
            info = tarfile.TarInfo(name)
            info.size = len(data)
            tf.addfile(info, io.BytesIO(data))
    ok, err = worlds.install_published_world("x", buf2.getvalue())
    check(not ok, "traversal member dropped → archive incomplete → rejected")


def test_exhibits_non_image_members_are_dropped():
    print("test_exhibits_non_image_members_are_dropped")
    worlds = fresh_env()
    import io
    import tarfile

    def archive(exhibit_name: str, exhibit_data: bytes) -> bytes:
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode="w:gz") as tf:
            for name, data in (
                ("world_meta.json", b"{}"),
                ("world.dat", b"d"),
                (f"exhibits/{exhibit_name}", exhibit_data),
            ):
                info = tarfile.TarInfo(name)
                info.size = len(data)
                tf.addfile(info, io.BytesIO(data))
        return buf.getvalue()

    # Wrong extension (2026-09-27 audit finding: any file type used to survive
    # into the exhibits dir Caddy serves same-origin with the admin cookie).
    blob = archive("x.html", b"<script>alert(1)</script>")
    ok, err = worlds.install_published_world("html-exhibit", blob)
    check(ok, "install still succeeds (world_meta.json + world.dat present)")
    dst = worlds.world_slot_dir("html-exhibit")
    check(not (dst / "exhibits" / "x.html").exists(), "non-image extension dropped, not installed")

    # Right extension, WRONG content (renamed .html masquerading as .png) — the
    # magic-byte sniff must catch what the extension allowlist alone can't.
    blob2 = archive("x.png", b"<script>alert(1)</script>")
    ok2, _ = worlds.install_published_world("fake-png-exhibit", blob2)
    check(ok2, "install still succeeds")
    dst2 = worlds.world_slot_dir("fake-png-exhibit")
    check(not (dst2 / "exhibits" / "x.png").exists(), "wrong-magic-bytes .png dropped, not installed")

    # Genuine PNG bytes with the right extension DO survive.
    real_png = b"\x89PNG\r\n\x1a\n" + b"real"
    blob3 = archive("ok.png", real_png)
    ok3, _ = worlds.install_published_world("real-png-exhibit", blob3)
    dst3 = worlds.world_slot_dir("real-png-exhibit")
    check(ok3 and (dst3 / "exhibits" / "ok.png").read_bytes() == real_png, "real PNG exhibit installs fine")


def test_dot_world_id_cannot_touch_the_volume():
    print("test_dot_world_id_cannot_touch_the_volume")
    worlds = fresh_env()
    import identity
    import io
    import tarfile
    # `.` and `..` are charset-legal but path-special — they must never validate,
    # because pathlib collapses `<worlds>/.` back to the worlds root and an install
    # would rename/rmtree the whole volume instead of one slot.
    check(not worlds.is_valid_world_id("."), "`.` rejected by validator")
    check(not worlds.is_valid_world_id(".."), "`..` rejected by validator")
    check(worlds.is_valid_world_id("server-world"), "ordinary id still accepted")

    # Seed the volume with a sibling world + the identity dir, then attempt a
    # fully-formed `.` publish. It must be refused with no collateral damage.
    worlds_root = identity.identity_dir().parent
    identity.identity_dir().mkdir(parents=True, exist_ok=True)
    sentinel = worlds_root / "keep-me"
    sentinel.mkdir(parents=True, exist_ok=True)
    (sentinel / "world.dat").write_bytes(b"precious")

    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tf:
        for name, data in (("world_meta.json", b"{}"), ("world.dat", b"d")):
            info = tarfile.TarInfo(name)
            info.size = len(data)
            tf.addfile(info, io.BytesIO(data))
    ok, err = worlds.install_published_world(".", buf.getvalue())
    check(not ok and err == "bad-world-id", "`.` publish rejected as bad-world-id")
    check((sentinel / "world.dat").read_bytes() == b"precious",
          "sibling world untouched after rejected `.` publish")
    check(identity.identity_dir().exists(), "identity dir untouched")


def main():
    sys.path.insert(0, str(Path(__file__).parent))
    for t in (test_empty_manifest_is_safe, test_record_and_get,
              test_upsert_preserves_created_unix, test_record_requires_id,
              test_remove, test_malformed_manifest_is_tolerated,
              test_pack_world_dir, test_pack_missing_or_unsafe_world_is_none,
              test_install_published_world_round_trip,
              test_install_rejects_incomplete_and_unsafe,
              test_exhibits_non_image_members_are_dropped,
              test_pop_secret_stripped_on_export_and_fresh_on_import,
              test_dot_world_id_cannot_touch_the_volume):
        t()
    print()
    if _fails:
        print(f"{_fails} check(s) FAILED")
        sys.exit(1)
    print("all worlds checks passed")


if __name__ == "__main__":
    main()
