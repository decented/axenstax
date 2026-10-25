"""The /more-games page lists Stampede and Twang Thang with APK downloads.

Run: cd tools/sites/marketing && .venv/bin/python -m pytest test_more_games.py -q
"""

import os
import sys

from starlette.testclient import TestClient

sys.path.insert(0, os.path.dirname(__file__))

import app as appmod  # noqa: E402

client = TestClient(appmod.app, raise_server_exceptions=True)


def test_more_games_page_exists():
    assert client.get("/more-games").status_code == 200


def test_more_games_lists_both_apks_with_their_checksums():
    text = client.get("/more-games").text
    for game in appmod.MORE_GAMES:
        assert game["name"] in text
        assert game["url"] in text
        assert game["sha256"] in text
        assert game["url"].endswith(game["sha256"] + ".apk")
        assert game["version"] in text
    assert "Download APK" in text


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
