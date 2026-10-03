"""Regression test for the 2026-09-28 pre-public audit fix (go-live MUST #20):
a /privacy page must exist, describe only what the code does today, and carry
a visible DRAFT banner until the owner signs off on the content.

Run: cd tools/sites/marketing && .venv/bin/python -m pytest test_privacy.py -q
"""

import os
import sys

from starlette.testclient import TestClient

sys.path.insert(0, os.path.dirname(__file__))

import app as appmod  # noqa: E402

client = TestClient(appmod.app, raise_server_exceptions=True)


def test_privacy_page_exists():
    r = client.get("/privacy")
    assert r.status_code == 200


def test_privacy_page_has_draft_banner():
    r = client.get("/privacy")
    assert "DRAFT" in r.text
    assert "pending owner sign-off" in r.text


def test_privacy_page_covers_every_data_flow_named_in_the_audit():
    r = client.get("/privacy")
    text = r.text
    # Area 5 of the 2026-09-28 audit: feedback, /mc-skin, the
    # cookie-free web game, and the native app's relay use must each be named.
    assert "/bug" in text or "feedback" in text.lower()
    assert "mojang" in text.lower() or "mc-skin" in text.lower()
    assert "cookie" in text.lower()
    assert "relay" in text.lower()


def test_every_marketing_page_footer_links_privacy():
    for path in ("/", "/roadmap", "/experiences", "/support", "/safety", "/will-it-run"):
        r = client.get(path)
        assert r.status_code == 200, path
        assert 'href="/privacy"' in r.text, f"{path} footer is missing a /privacy link"


def test_privacy_page_says_browser_has_no_feedback_channel():
    """2026-10-01 owner decision: the web taster has no /bug, /idea or mailbox.
    The page must not claim web feedback exists, and must say it doesn't."""
    text = client.get("/privacy").text
    assert "no feedback channel" in text
    # The only feedback wording left is explicitly scoped to the desktop app.
    assert "Feedback in the desktop app" in text
    assert "web version never asks" not in text


def test_privacy_page_says_we_do_not_log_ip_addresses():
    text = client.get("/privacy").text
    assert "not log IP addresses" in text
    assert "nginx" not in text.lower()


def test_privacy_page_discloses_web_server_logging():
    """2026-10-03: the proxy keeps no access logs; a failed proxied request can
    write an error line, but the box masks IPs to /24 and keeps 3 days. The page
    must say so, not just "apps don't log"."""
    text = client.get("/privacy").text
    assert "no access logs" in text
    assert "never your full IP address" in text
    assert "deletes entries after 3 days" in text


def test_contact_form_is_gone():
    """2026-10-03: the contact form was removed entirely. /contact must 404 for
    both GET and POST, and no page may link to it or describe it."""
    assert client.get("/contact").status_code == 404
    assert client.post("/contact", data={"name": "x", "contact": "a@b.co"}).status_code in (404, 405)
    for path in ("/", "/roadmap", "/experiences", "/support", "/safety", "/will-it-run", "/privacy"):
        text = client.get(path).text
        assert 'href="/contact"' not in text, path
    assert "contact form" not in client.get("/privacy").text.lower()


def test_privacy_page_has_the_uk_gdpr_notice_essentials():
    """2026-10-03 legal pass (UK GDPR Art 13, DPA 2018 s.164A as inserted by the
    Data (Use and Access) Act 2025, ICO Children's Code std 4): who the
    controller is + a contact route, a lawful basis and retention per item, the
    right to object stated separately, complaints to us first then the ICO, a
    child-friendly summary, and a last-updated date."""
    text = client.get("/privacy").text
    for needle in (
        "Who we are",
        "controller",
        "Contact:",
        "Legal basis",
        "How long",
        "Your right to object",
        "Complaints",
        "ico.org.uk/make-a-complaint",
        "within 30 days",
        "for players of any age",
        "Last updated:",
    ):
        assert needle in text, needle


def test_privacy_page_does_not_overstate_what_reports_or_the_web_game_send():
    """Reports carry only message, kind, a random report id and the build
    (native_mailbox::wire::build_rumor); the web crash reporter is gone."""
    text = client.get("/privacy").text
    assert "position in the world" not in text
    assert "technical error message" not in text
