"""Regression test for the 2026-09-28 pre-public audit fix (go-live MUST #20):
a /privacy page must exist, describe only what the code does today, and carry
no DRAFT banner now the owner has signed it off (2026-10-04).

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


def test_privacy_page_is_signed_off():
    r = client.get("/privacy")
    assert "DRAFT" not in r.text
    assert "pending owner sign-off" not in r.text
    assert "decented" in r.text
    assert "no longer than 6 months" not in r.text


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
    for path in ("/", "/roadmap", "/experiences", "/more-games", "/support", "/safety", "/will-it-run"):
        r = client.get(path)
        assert r.status_code == 200, path
        assert 'href="/privacy"' in r.text, f"{path} footer is missing a /privacy link"


def test_privacy_page_says_browser_has_no_feedback_channel():
    """2026-10-01 owner decision: the web taster has no /bug, /idea or mailbox.
    The page must not claim web feedback exists, and must say it doesn't."""
    text = client.get("/privacy").text
    assert "no feedback channel" in text
    assert "web version never asks" not in text
    assert "Feedback in the browser" not in text
    assert "Feedback in the desktop app" not in text


def test_privacy_page_scopes_claims_by_desktop_version():
    """2026-11-02: the page ships before the next release, so it must be true
    while 0.2.18 is the public download and after the next release."""
    text = client.get("/privacy").text
    assert "Desktop version 0.2.18, still the public download, and the next release behave differently" in text
    assert "complete list for the game and its sites" in text
    assert "carries its own notice" in text
    assert "If it isn't here, we don't collect it" not in text
    assert "Last updated: 2 November 2026" in text


def test_privacy_page_feedback_is_ended_and_says_what_0_2_18_did():
    """Verified against git 4d445526 (v0.2.18): no on/off gate, one relay
    (relay.trotters.cc), a stored device key, and a report holds the message,
    its type, a report id, a client label and the public key if signed in."""
    text = client.get("/privacy").text
    assert "Bug and idea reports sent from desktop 0.2.18 or earlier" in text
    assert "until we ended this on 1 November 2026" in text
    assert "We stopped reading reports and deleted the ones we held on 1 November 2026" in text
    assert "Encrypted copies may remain on the relays that carried them" in text
    assert "in that version they are on by default" in text
    assert "Nothing sent now will be read" in text
    assert "device key" in text
    assert "/mailbox" not in text
    assert "adult testers" not in text
    assert "off by default" not in text
    assert "ask us to delete reports" not in text


def test_privacy_page_discloses_the_relay_we_run_for_0_2_18():
    text = client.get("/privacy").text
    assert "Your IP address and public key, on the relay we run (desktop 0.2.18 only)" in text
    assert "relay.trotters.cc" in text
    assert "sign-in, server cards and bug and idea reports" in text
    assert "Newer releases do not use this relay by default" in text
    assert "We are confirming how long this relay" in text
    assert "connection setup" not in text.lower()


def test_privacy_page_skin_rows_match_the_sweep_timer():
    """tools/sites/game/app.py sweeps every 60 s: a rate bucket (60 s window)
    goes within ~2 minutes, a username (300 s TTL) within ~6."""
    text = client.get("/privacy").text
    assert "Cleared within about two minutes of your lookup" in text
    assert "Kept in memory for five minutes, then cleared within the next minute" in text
    assert "Gone within minutes" not in text


def test_privacy_page_desktop_card_is_accurate_for_the_next_release():
    text = client.get("/privacy").text
    assert "Desktop 0.2.18 instead uses <code>relay.trotters.cc</code> for sign-in and has no online play" in text
    assert "which public keys a message is addressed to, and when you check for messages" in text
    assert "whenever you host or join online, including Host Game" in text
    assert "Never an email or birth date" in text
    assert "Never a name" not in text
    assert "a signed display name can also reach the game" in text.lower()
    assert "Other players." in text
    assert "they (or their router) can see your IP address" in text
    assert "AXENSTAX_NO_UPDATE_CHECK=1" in text
    assert "only when you play online" not in text


def test_privacy_page_names_no_contact_address():
    """The owner has not supplied a contact route; none may be invented."""
    text = client.get("/privacy").text
    assert "mailto:" not in text
    assert "@" not in text.replace("@media", "")


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
    controller is (contact route deferred — see internal compliance actions), a lawful basis and retention per item, the
    right to object stated separately, complaints to us first then the ICO, a
    child-friendly summary, and a last-updated date."""
    text = client.get("/privacy").text
    for needle in (
        "Who we are",
        "controller",
        "Legal basis",
        "How long",
        "Your right to object",
        "Complaints",
        "ico.org.uk/make-a-complaint",
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


def test_every_marketing_page_except_roadmap_links_more_games():
    """The owner is editing roadmap.html, so it is the one page left out."""
    for path in ("/", "/experiences", "/more-games", "/support", "/safety", "/will-it-run", "/privacy"):
        r = client.get(path)
        assert r.status_code == 200, path
        assert 'href="/more-games"' in r.text, f"{path} is missing a /more-games link"
