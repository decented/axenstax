"""The /more-games page lists Stampede, Twang Thang and Cluster Flock with APK downloads.

Run: cd tools/sites/marketing && .venv/bin/python -m pytest test_more_games.py -q
"""

import asyncio
import hashlib
import os
import sys
from unittest.mock import patch

from starlette.testclient import TestClient

sys.path.insert(0, os.path.dirname(__file__))

import app as appmod  # noqa: E402

client = TestClient(appmod.app, raise_server_exceptions=True)


def test_more_games_page_exists():
    assert client.get("/more-games").status_code == 200


def test_more_games_lists_every_apk_with_their_checksums():
    text = client.get("/more-games").text
    for game in appmod.MORE_GAMES:
        assert game["name"] in text
        assert f'href="/more-games/{game["name"].lower().replace(" ", "-")}.apk"' in text
        assert game["url"] not in text  # buttons point at this site; the public host is only the fallback
        assert game["sha256"] in text
        assert game["url"].endswith(game["sha256"] + ".apk")
        assert game["version"] in text
    assert "Download APK" in text


def test_more_games_lists_the_three_games_with_anchors_the_lobby_links_to():
    text = client.get("/more-games").text
    assert [g["name"] for g in appmod.MORE_GAMES] == ["Stampede", "Twang Thang", "Cluster Flock"]
    for slug in ("stampede", "twang-thang", "cluster-flock"):
        assert text.count(f'id="{slug}"') == 1, slug


def test_more_games_footer_links_privacy():
    assert 'href="/privacy"' in client.get("/more-games").text


def test_more_games_icons_are_served():
    for game in appmod.MORE_GAMES:
        r = client.get(game["icon"])
        assert r.status_code == 200
        assert r.headers["content-type"] == "image/png"


def test_every_marketing_page_links_more_games_in_its_nav_and_footer():
    for path in ("/", "/will-it-run", "/experiences", "/roadmap", "/safety", "/support", "/privacy", "/more-games"):
        text = client.get(path).text
        assert text.count('href="/more-games"') == 2, path


def _catalogue_with_files(tmp_path, store=True):
    cat = appmod.ReleaseCatalogue(appmod.MORE_GAMES, tmp_path / "data" / "more-games.json")
    bodies = {}
    for g in cat.view():
        body = f"apk for {g['name']}".encode()
        # Same-size stand-ins so the stored file matches the announced size.
        cat.current[g["package"]].update(size_bytes=len(body), sha256=hashlib.sha256(body).hexdigest())
        bodies[g["name"]] = body
        if store:
            cat.apk_dir.mkdir(parents=True, exist_ok=True)
            (cat.apk_dir / f"{hashlib.sha256(body).hexdigest()}.apk").write_bytes(body)
    if store:
        asyncio.run(cat.verify_stored())  # what refresh() does off the request path
    return cat, bodies


def test_apk_route_serves_the_stored_file_with_android_headers(tmp_path):
    cat, bodies = _catalogue_with_files(tmp_path)
    names = {"stampede": ("Stampede", "Stampede-0.4.14.apk"),
             "twang-thang": ("Twang Thang", "TwangThang-0.8.18.apk"),
             "cluster-flock": ("Cluster Flock", "ClusterFlock-0.2.1.apk")}
    with patch.object(appmod, "catalogue", cat):
        for slug, (name, filename) in names.items():
            r = client.get(f"/more-games/{slug}.apk")
            assert r.status_code == 200, slug
            assert r.headers["content-type"] == "application/vnd.android.package-archive"
            assert r.headers["content-disposition"] == f'attachment; filename="{filename}"'
            assert r.headers["x-content-type-options"] == "nosniff"
            assert r.headers["cache-control"] == "no-cache"
            assert r.content == bodies[name]
        head = client.head("/more-games/twang-thang.apk")
        assert head.status_code == 200 and head.content == b""
        assert head.headers["content-type"] == "application/vnd.android.package-archive"


def test_apk_route_redirects_to_the_public_host_until_the_file_is_stored(tmp_path):
    cat, _ = _catalogue_with_files(tmp_path, store=False)
    with patch.object(appmod, "catalogue", cat):
        r = client.get("/more-games/stampede.apk", follow_redirects=False)
    assert r.status_code == 302
    assert r.headers["location"] == cat.view()[0]["url"]


def test_apk_route_unknown_slug_is_404(tmp_path):
    cat, _ = _catalogue_with_files(tmp_path)
    with patch.object(appmod, "catalogue", cat):
        for slug in ("nope", "Stampede", "stampede.apk", "..%2Fx"):
            assert client.get(f"/more-games/{slug}.apk").status_code == 404, slug


def test_apk_filename_cannot_carry_header_characters_from_a_signed_version(tmp_path):
    cat, _ = _catalogue_with_files(tmp_path)
    cat.current[cat.view()[0]["package"]]["version"] = 'a"; x=\\y/z'
    with patch.object(appmod, "catalogue", cat):
        r = client.get("/more-games/stampede.apk")
    assert r.headers["content-disposition"] == 'attachment; filename="Stampede-a___x__y_z.apk"'


def test_page_links_each_button_to_this_site_and_names_the_source(tmp_path):
    text = client.get("/more-games").text
    for slug in ("stampede", "twang-thang", "cluster-flock"):
        assert f'href="/more-games/{slug}.apk"' in text
    assert "Downloads come from this site, checked against each game&#39;s signed release." in text or \
        "Downloads come from this site, checked against each game's signed release." in text
    assert "The same files are published on nostr.download, a public file host." in text


def test_a_symlink_or_wrong_hash_file_is_never_served_and_a_vanished_one_falls_back(tmp_path):
    cat, bodies = _catalogue_with_files(tmp_path)
    stampede = cat.view()[0]
    path = cat.apk_dir / f"{stampede['sha256']}.apk"
    with patch.object(appmod, "catalogue", cat):
        assert client.get("/more-games/stampede.apk").status_code == 200
        path.unlink()  # vanished: a redirect, not a 500
        assert client.get("/more-games/stampede.apk", follow_redirects=False).status_code == 302
        target = tmp_path / "elsewhere.apk"
        target.write_bytes(bodies["Stampede"])
        path.symlink_to(target)  # same bytes, but a link
        assert client.get("/more-games/stampede.apk", follow_redirects=False).status_code == 302
        path.unlink()
        path.write_bytes(b"X" * len(bodies["Stampede"]))  # right size, wrong bytes, never verified
        assert client.get("/more-games/stampede.apk", follow_redirects=False).status_code == 302
