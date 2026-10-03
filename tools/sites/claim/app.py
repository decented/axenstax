#!/usr/bin/env python3
"""Axe'n'Stax — merch claim / fulfilment intake (port 8100).

The booth model: you take payment IN PERSON (cash or sats to your own wallet) or
award a prize, and hand over a single-use **code**. The buyer scans a QR to this
page, enters the code, picks a size, and types **their own** shipping address.
This pushes the order to Printful, which prints and ships it — you never type
their address or estimate shipping. There is NO payment here by design.

Safety: the form is useless without a valid unused code, so a stray visitor can't
run up your Printful bill. Orders are created as **drafts by default**
(PRINTFUL_AUTO_CONFIRM=false) — Printful neither charges nor ships until you
confirm in its dashboard. Flip auto-confirm on once you trust the flow.

Lives on claim.axenstax.com in production. See README.md.
"""

import logging
import os
import re
import time
from pathlib import Path

from dotenv import load_dotenv
from fastapi import FastAPI, Form, Request
from fastapi.responses import HTMLResponse
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates

import codes
import printful

load_dotenv()
log = logging.getLogger(__name__)

BASE_DIR = Path(__file__).parent
CERTS_DIR = BASE_DIR / "certs"

PORT = int(os.environ.get("PORT", "8100"))
AUTO_CONFIRM = os.environ.get("PRINTFUL_AUTO_CONFIRM", "false").strip().lower() in ("1", "true", "yes")
SUPPORT_URL = os.environ.get("MARKETING_URL", "https://localhost:8096")
VARIANT_TTL = 300  # seconds
_EMAIL_RE = re.compile(r"^[^@\s]+@[^@\s]+\.[^@\s]+$")
_STATE_REQUIRED = {"US", "CA", "AU"}

app = FastAPI(title="Axe'n'Stax — Claim")

_BASELINE_CSP = (
    "default-src 'self'; "
    "script-src 'self'; "
    "style-src 'self' 'unsafe-inline'; "
    "connect-src 'self'; "
    "img-src 'self' data:; "
    "font-src 'self' data:; "
    "object-src 'none'; "
    "base-uri 'self'; "
    "form-action 'self'; "
    "frame-src 'none'; "
    "frame-ancestors 'none'"
)


@app.middleware("http")
async def _add_security_headers(request, call_next):
    response = await call_next(request)
    response.headers.setdefault("X-Frame-Options", "DENY")
    response.headers.setdefault("X-Content-Type-Options", "nosniff")
    response.headers.setdefault("Referrer-Policy", "no-referrer")
    scheme = request.url.scheme or ""
    forwarded = request.headers.get("x-forwarded-proto", "").lower()
    if scheme == "https" or forwarded == "https":
        response.headers.setdefault(
            "Strict-Transport-Security", "max-age=31536000; includeSubDomains"
        )
    if "text/html" in response.headers.get("content-type", "").lower():
        response.headers.setdefault("Content-Security-Policy", _BASELINE_CSP)
    return response


app.mount("/static", StaticFiles(directory=str(BASE_DIR / "static")), name="static")
templates = Jinja2Templates(directory=str(BASE_DIR / "templates"))

# --- Variant cache (avoid an N+1 Printful round-trip on every page view) ---
_cache: dict = {"at": 0.0, "data": None}


def get_variants():
    """Cached list of synced variants, or None if unconfigured / Printful errored."""
    if not printful.configured():
        return None
    now = time.time()
    if _cache["data"] is not None and (now - _cache["at"]) < VARIANT_TTL:
        return _cache["data"]
    try:
        data = printful.list_variants()
    except printful.PrintfulError as e:
        log.warning("Printful variant fetch failed: %s", e)
        return _cache["data"]  # fall back to stale cache if we have one
    _cache["data"], _cache["at"] = data, now
    return data


def _msg(request, title, body, *, tone="info", status_code=200):
    return templates.TemplateResponse(
        request=request, name="message.html",
        context={"title": title, "body": body, "tone": tone, "support_url": SUPPORT_URL},
        status_code=status_code,
    )


@app.get("/", response_class=HTMLResponse)
async def claim_form(request: Request, code: str = ""):
    if not printful.configured():
        return _msg(request, "Not open yet",
                    "The merch claim isn't switched on yet. Check back shortly.")
    variants = get_variants()
    if not variants:
        return _msg(request, "Just a moment",
                    "We can't reach the print service right now. Please try again in a minute.")
    if code and codes.status(code) == "used":
        return _msg(request, "Already claimed",
                    "This code has already been used to place an order. If that wasn't you, get in touch.",
                    tone="warn")
    return templates.TemplateResponse(
        request=request, name="claim.html",
        context={"variants": variants, "code": codes.normalise(code)},
    )


@app.post("/claim", response_class=HTMLResponse)
async def submit_claim(
    request: Request,
    code: str = Form(""),
    sync_variant_id: str = Form(""),
    name: str = Form(""),
    address1: str = Form(""),
    address2: str = Form(""),
    city: str = Form(""),
    state_code: str = Form(""),
    country_code: str = Form(""),
    zip: str = Form(""),
    email: str = Form(""),
    phone: str = Form(""),
):
    if not printful.configured():
        return _msg(request, "Not open yet", "The merch claim isn't switched on yet.")
    variants = get_variants() or []
    valid_ids = {str(v.sync_variant_id) for v in variants}

    # --- validate before touching the code or Printful ---
    country = (country_code or "").strip().upper()[:2]
    errors = []
    if sync_variant_id not in valid_ids:
        errors.append("Please choose an item.")
    for label, val in [("name", name), ("address line 1", address1), ("town/city", city),
                       ("country", country), ("postcode/ZIP", zip), ("email", email)]:
        if not (val or "").strip():
            errors.append(f"Please fill in your {label}.")
    if email and not _EMAIL_RE.match(email.strip()):
        errors.append("That email doesn't look right.")
    if country in _STATE_REQUIRED and not (state_code or "").strip():
        errors.append("A state/province is required for your country.")
    if errors:
        return templates.TemplateResponse(
            request=request, name="claim.html",
            context={"variants": variants, "code": codes.normalise(code),
                     "errors": errors, "form": {
                         "name": name, "address1": address1, "address2": address2,
                         "city": city, "state_code": state_code, "country_code": country,
                         "zip": zip, "email": email, "phone": phone,
                         "sync_variant_id": sync_variant_id}},
            status_code=400,
        )

    # --- reserve the code (atomic, single-use) BEFORE creating the order ---
    if not codes.reserve(code):
        return _msg(request, "That code didn't work",
                    "It's either not a valid code or it's already been used. Double-check it, "
                    "or grab us at the stand.", tone="warn", status_code=400)

    recipient = {
        "name": name.strip(), "address1": address1.strip(), "city": city.strip(),
        "country_code": country, "zip": zip.strip(), "email": email.strip(),
    }
    if address2.strip():
        recipient["address2"] = address2.strip()
    if state_code.strip():
        recipient["state_code"] = state_code.strip()
    if phone.strip():
        recipient["phone"] = phone.strip()

    try:
        order = printful.create_order(int(sync_variant_id), recipient, confirm=AUTO_CONFIRM)
    except printful.PrintfulError as e:
        codes.release(code)  # give the code back so they can retry
        log.error("Order creation failed: %s", e)
        return _msg(request, "Something went wrong",
                    "We couldn't place the order just now — your code is still valid, please try "
                    "again in a minute. If it keeps failing, come back to the stand.",
                    tone="warn", status_code=502)

    codes.set_order(code, order.get("id"))
    return templates.TemplateResponse(
        request=request, name="done.html",
        context={"order_id": order.get("id"), "auto_confirm": AUTO_CONFIRM,
                 "support_url": SUPPORT_URL},
    )


if __name__ == "__main__":
    import uvicorn
    cert = CERTS_DIR / "cert.pem"
    key = CERTS_DIR / "key.pem"
    ssl_kwargs = {}
    if cert.exists() and key.exists():
        ssl_kwargs = {"ssl_certfile": str(cert), "ssl_keyfile": str(key)}
    uvicorn.run(app, host="0.0.0.0", port=PORT, log_level="info", access_log=False, **ssl_kwargs)
