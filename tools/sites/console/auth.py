"""Signet authentication endpoints for Axe'n'Stax PWA alpha.

Flow (signet-login-driven — all crypto happens in the browser):

    1. Client: POST /auth/challenge → {challenge}
       Server stores `challenge → expires_at` for CSRF binding.

    2. Client: calls window.Signet.login({ appName, challenge, relayUrl })
       (or window.Signet.handleRedirectCallback() on return from same-device).
       The browser handles QR + relay subscribe + same-device redirect.
       It resolves with a signed kind-21236 SignetAuthEvent.

    3. Client: POST /auth/verify  with `{authEvent}`.
       Server verifies the event (schnorr sig + challenge + origin + freshness),
       mints the axenstax_session cookie, returns {ok, pubkey}.

The old `/auth/callback`, `/auth/verify-fragment`, and `/auth/relay-complete`
endpoints — and the fragment-HMAC + multi-state session machinery they
required — were removed in the signet-login cutover (2026-05-20). See
docs/integrations/signet/2026-05-20-signet-login-adoption.md.

Signature verification uses BIP-340 Schnorr (secp256k1). HARD-FAILS at import
if the secp256k1 lib is unavailable — there is no silent fallback. The
previous format-only fallback was an auth bypass.
"""

import asyncio
import base64
import hashlib
import hmac as _hmac
import json
import logging
import re
import secrets
import time
from pathlib import Path

from fastapi import APIRouter, HTTPException, Request, Header
from fastapi.responses import JSONResponse
from fastapi.templating import Jinja2Templates
from pydantic import BaseModel, Field

# BIP-340 Schnorr signature verification via secp256k1.
# HARD-FAIL at import time — no silent fallback. Falling through with format-only
# verification would let any 64-hex pubkey + any 128-hex signature authenticate.
try:
    from secp256k1._libsecp256k1 import ffi as _ffi, lib as _secp_lib
    _secp_ctx = _ffi.gc(
        _secp_lib.secp256k1_context_create(0x0301),  # SIGN | VERIFY
        _secp_lib.secp256k1_context_destroy,
    )
    HAS_SCHNORR = True
except ImportError as exc:
    raise RuntimeError(
        "secp256k1 Schnorr library unavailable — refusing to start. "
        "Install with `pip install secp256k1` in the website venv."
    ) from exc


AUTH_EVENT_KIND = 21236
# Reserved for future use. signet-app's current `signAuthChallenge`
# (src/lib/signet.ts) signs the kind-21236 event with only `[challenge,
# origin]` (plus optional `avatar_*` tags) — no `app` tag. Setting this
# to None disables the `expected_app` check so verification succeeds.
# Reintroduce when signet-app starts emitting an `app` tag on the signed
# event (currently blocked upstream — same root cause as the redirect-
# reconstruction issue in signet-login/redirect.ts).
AUTH_EVENT_APP_NAME: str | None = None
AUTH_EVENT_CREATED_AT_SKEW = 300  # seconds — matches signet-app's freshness window
AUTH_EVENT_CLOCK_SKEW_TOLERANCE = 60  # seconds in the future (phone clock drift)

# Publish flow (build spec §3) — the operator-signed publish-authorization event.
# The game builds + signs it with the operator key to authorise a world publish;
# the console verifies it instead of a browser cookie (the game is not the
# console browser, so it carries its own proof in the request).
PUBLISH_AUTHZ_KIND = 27490
PUBLISH_AUTHZ_SKEW = 300  # ± seconds — symmetric freshness window, mirrors admin.rs

HEX_RE = re.compile(r"^[0-9a-f]+$")


def _is_hex(s: str, length: int) -> bool:
    return isinstance(s, str) and len(s) == length and bool(HEX_RE.match(s.lower()))


def _nostr_event_id(pubkey: str, created_at: int, kind: int, tags: list, content: str) -> str:
    """Compute the NIP-01 canonical event ID (hex SHA-256)."""
    payload = json.dumps(
        [0, pubkey, created_at, kind, tags, content],
        separators=(",", ":"),
        ensure_ascii=False,
    )
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def _schnorr_verify_raw(pubkey_hex: str, signature_hex: str, msg32: bytes) -> bool:
    """BIP-340 Schnorr verify on a 32-byte message."""
    try:
        pubkey_bytes = bytes.fromhex(pubkey_hex)
        sig_bytes = bytes.fromhex(signature_hex)
        xonly_pk = _ffi.new("secp256k1_xonly_pubkey *")
        if _secp_lib.secp256k1_xonly_pubkey_parse(_secp_ctx, xonly_pk, pubkey_bytes) != 1:
            return False
        return _secp_lib.secp256k1_schnorrsig_verify(
            _secp_ctx, sig_bytes, msg32, 32, xonly_pk
        ) == 1
    except Exception:
        return False


def verify_signet_auth_event(
    auth_event: dict,
    expected_challenge: str,
    expected_origin: str,
    expected_app: str | None = AUTH_EVENT_APP_NAME,
) -> tuple[str | None, str | None]:
    """Verify a Signet kind-21236 auth event delivered by signet-login.

    Returns (pubkey_hex_lower, None) on success or (None, error_token) on failure.

    Checks performed (mirrors signet-login/src/verify.ts, the authoritative
    reference):

      1. Shape / required fields
      2. kind == 21236
      3. Event ID is SHA-256 of NIP-01 canonical serialisation
      4. BIP-340 Schnorr signature verifies the event ID
      5. `challenge` tag matches the one we issued
      6. `origin` tag matches our origin
      7. `app` tag matches AUTH_EVENT_APP_NAME (optional but enforced when set)
      8. Freshness — created_at within ±skew of now
    """
    if not isinstance(auth_event, dict):
        return None, "malformed-event"
    required = {"id", "pubkey", "sig", "kind", "created_at", "tags", "content"}
    if not required.issubset(auth_event):
        return None, "malformed-event"
    if auth_event["kind"] != AUTH_EVENT_KIND:
        return None, "wrong-kind"

    pubkey = auth_event["pubkey"]
    if not _is_hex(pubkey, 64):
        return None, "bad-pubkey"
    pubkey = pubkey.lower()

    sig = auth_event["sig"]
    if not _is_hex(sig, 128):
        return None, "bad-sig"
    sig = sig.lower()

    event_id = auth_event["id"]
    if not _is_hex(event_id, 64):
        return None, "bad-event-id"
    event_id = event_id.lower()

    created_at = auth_event["created_at"]
    if not isinstance(created_at, int) or isinstance(created_at, bool):
        return None, "bad-created-at"

    tags = auth_event["tags"]
    if not isinstance(tags, list):
        return None, "bad-tags"

    content = auth_event["content"]
    if not isinstance(content, str):
        return None, "bad-content"

    # Recompute event ID — proves the (pubkey, created_at, tags, content) tuple
    # we just verified the sig over really did hash to the id Signet signed.
    computed_id = _nostr_event_id(pubkey, created_at, AUTH_EVENT_KIND, tags, content)
    if computed_id != event_id:
        return None, "invalid-event-id"

    # Schnorr verify
    try:
        msg32 = bytes.fromhex(event_id)
    except ValueError:
        return None, "bad-event-id"
    if not _schnorr_verify_raw(pubkey, sig, msg32):
        return None, "invalid-signature"

    # Extract challenge / origin / app tags (first wins; tag shape is [name, value, ...]).
    challenge_tag: str | None = None
    origin_tag: str | None = None
    app_tag: str | None = None
    for tag in tags:
        if not isinstance(tag, list) or len(tag) < 2:
            continue
        if not isinstance(tag[0], str) or not isinstance(tag[1], str):
            continue
        if tag[0] == "challenge" and challenge_tag is None:
            challenge_tag = tag[1]
        elif tag[0] == "origin" and origin_tag is None:
            origin_tag = tag[1]
        elif tag[0] == "app" and app_tag is None:
            app_tag = tag[1]

    if not challenge_tag or challenge_tag.lower() != expected_challenge.lower():
        return None, "challenge-mismatch"
    if origin_tag != expected_origin:
        return None, "origin-mismatch"
    if expected_app is not None and app_tag != expected_app:
        return None, "app-mismatch"

    # Freshness — caught between two clocks, allow some drift either way.
    age = time.time() - created_at
    if age > AUTH_EVENT_CREATED_AT_SKEW:
        return None, "too-old"
    if age < -AUTH_EVENT_CLOCK_SKEW_TOLERANCE:
        return None, "in-the-future"

    return pubkey, None


def verify_publish_authorization(
    event: dict,
    expected_world_id: str,
    expected_archive_sha256: str,
    operator_pubkeys_hex,
    now: float | None = None,
) -> tuple[str | None, str | None]:
    """Verify an operator-signed publish-authorization event (build spec §3).

    The game packs a `.axeworld`, hashes it, and signs a kind-`PUBLISH_AUTHZ_KIND`
    Nostr event binding `{world, x=archive_sha256, nonce}` with the operator's key.
    The console accepts the upload ONLY if every check holds:

      1. shape ok + kind == PUBLISH_AUTHZ_KIND
      2. event id == NIP-01 hash of (pubkey, created_at, kind, tags, content)
      3. BIP-340 schnorr sig verifies that id
      4. pubkey is one of THIS server's operators (the publish gate)
      5. the `world` tag == the target world id (no cross-world publish)
      6. the `x` (hash) tag == SHA-256 of the uploaded bytes (anti-swap)
      7. created_at within ±PUBLISH_AUTHZ_SKEW of now (anti-replay)

    Returns (operator_pubkey_hex_lower, None) on success, else (None, error_token).
    The signature is checked BEFORE the bound-field checks so a forged event can
    never reach the (cheaper) tag comparisons with a trusted pubkey.
    """
    if now is None:
        now = time.time()
    if not isinstance(event, dict):
        return None, "malformed-event"
    required = {"id", "pubkey", "sig", "kind", "created_at", "tags", "content"}
    if not required.issubset(event):
        return None, "malformed-event"
    if event["kind"] != PUBLISH_AUTHZ_KIND:
        return None, "wrong-kind"

    pubkey = event["pubkey"]
    if not _is_hex(pubkey, 64):
        return None, "bad-pubkey"
    pubkey = pubkey.lower()

    sig = event["sig"]
    if not _is_hex(sig, 128):
        return None, "bad-sig"
    sig = sig.lower()

    event_id = event["id"]
    if not _is_hex(event_id, 64):
        return None, "bad-event-id"
    event_id = event_id.lower()

    created_at = event["created_at"]
    if not isinstance(created_at, int) or isinstance(created_at, bool):
        return None, "bad-created-at"

    tags = event["tags"]
    if not isinstance(tags, list):
        return None, "bad-tags"
    content = event["content"]
    if not isinstance(content, str):
        return None, "bad-content"

    # Recompute the id — proves the (pubkey, created_at, tags, content) tuple we
    # verify the sig over is the one that hashed to the signed id.
    computed_id = _nostr_event_id(pubkey, created_at, PUBLISH_AUTHZ_KIND, tags, content)
    if computed_id != event_id:
        return None, "invalid-event-id"

    try:
        msg32 = bytes.fromhex(event_id)
    except ValueError:
        return None, "bad-event-id"
    if not _schnorr_verify_raw(pubkey, sig, msg32):
        return None, "invalid-signature"

    # The signer must be an operator of THIS server — the publish gate.
    operators = {p.lower() for p in operator_pubkeys_hex}
    if pubkey not in operators:
        return None, "not-operator"

    # Bound fields (first matching tag wins; tag shape is [name, value, ...]).
    world_tag: str | None = None
    hash_tag: str | None = None
    for tag in tags:
        if not isinstance(tag, list) or len(tag) < 2:
            continue
        if not isinstance(tag[0], str) or not isinstance(tag[1], str):
            continue
        if tag[0] == "world" and world_tag is None:
            world_tag = tag[1]
        elif tag[0] == "x" and hash_tag is None:
            hash_tag = tag[1]

    if world_tag != expected_world_id:
        return None, "world-mismatch"
    if not hash_tag or hash_tag.lower() != expected_archive_sha256.lower():
        return None, "hash-mismatch"

    age = now - created_at
    if age > PUBLISH_AUTHZ_SKEW:
        return None, "too-old"
    if age < -PUBLISH_AUTHZ_SKEW:
        return None, "in-the-future"

    return pubkey, None


log = logging.getLogger(__name__)
router = APIRouter(prefix="/auth", tags=["auth"])

# Pending challenges we've issued — small dict keyed by challenge hex,
# value = expiry epoch. The server-issued challenge in `/auth/challenge`
# is what binds the eventual /auth/verify call to a request we initiated
# (CSRF protection). Without this binding, the client could mint its own
# challenge, get Signet to sign it, and post a valid event we never asked for.
_pending_challenges: dict[str, int] = {}
_pending_lock = asyncio.Lock()

CHALLENGE_TTL = 300        # 5 minutes — matches Signet's auth event freshness window
MAX_PENDING_CHALLENGES = 200  # cap to prevent memory exhaustion

COOKIE_NAME = "axenstax_console_session"
COOKIE_TTL = 90 * 24 * 3600  # 90 days — sliding window, re-minted on each /auth/whoami
                             # (Spec 32 Phase A sticky session). An active player never
                             # re-auths; an abandoned device lapses after 90 days idle.

templates: Jinja2Templates | None = None
_hmac_secret: bytes | None = None
_hmac_key_path: Path | None = None


def configure(tpl: Jinja2Templates, data_dir: Path):
    """Called from app.py on startup."""
    global templates, _hmac_secret, _hmac_key_path
    templates = tpl
    _hmac_key_path = data_dir / "fragment_hmac.key"  # file is reused — still a 32-byte HMAC secret
    _hmac_secret = _load_or_create_hmac_secret(_hmac_key_path)


def _load_or_create_hmac_secret(path: Path) -> bytes:
    """Read the 32-byte HMAC secret from disk, or mint a fresh one if absent.

    Never regenerates on subsequent boots — a corrupt file aborts startup.
    Same secret backs the session cookie HMAC. (Was also fragment-token HMAC
    pre-signet-login; that audience is no longer issued.)
    """
    if path.exists():
        data = path.read_bytes()
        if len(data) != 32:
            raise RuntimeError(
                f"HMAC key at {path} is corrupt (got {len(data)} bytes, expected 32). "
                "Delete and restart to mint a fresh secret; all existing sessions will invalidate."
            )
        return data
    path.parent.mkdir(parents=True, exist_ok=True)
    secret = secrets.token_bytes(32)
    path.write_bytes(secret)
    path.chmod(0o600)
    log.info(f"Minted fresh HMAC secret at {path}")
    return secret


def _request_is_https(request: Request) -> bool:
    """True if the request arrived via TLS — direct HTTPS or via a trusted
    reverse proxy asserting `X-Forwarded-Proto: https`. Used to decide whether
    the `Secure` cookie flag is safe to set: Chromium silently refuses Secure
    cookies on plain HTTP and the auth flow would then fail invisibly."""
    if (request.url.scheme or "").lower() == "https":
        return True
    return request.headers.get("x-forwarded-proto", "").lower() == "https"


def _request_origin(request: Request) -> str:
    """Return the canonical origin for this request, upgrading http→https for
    non-loopback hosts so the `origin` tag check matches what signet-login
    sent (signet-login uses `window.location.origin`, which is the https URL
    served by the dev cert)."""
    origin = str(request.base_url).rstrip("/")
    if origin.startswith("http://") and "localhost" not in origin:
        origin = "https://" + origin[7:]
    return origin


def _b64url(b: bytes) -> str:
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode("ascii")


def _b64url_decode(s: str) -> bytes:
    pad = (-len(s)) % 4
    return base64.urlsafe_b64decode(s + ("=" * pad))


# Audience separator — prefix every HMAC input with the exact use-case so
# tokens can never cross-verify. Currently only one audience (cookie) since
# the fragment-token audience went away with signet-login adoption.
_AUD_COOKIE = b"v1:cookie:"


def _hmac_mac(
    aud: bytes,
    pubkey: str,
    expires_ts: int,
    np_flag: str | None = None,
    handle_b64: str | None = None,
) -> bytes:
    assert _hmac_secret is not None, "auth.configure() must be called at startup"
    parts = [pubkey, str(expires_ts)]
    if np_flag is not None:
        parts.append(np_flag)
    if handle_b64 is not None:
        parts.append(handle_b64)
    body = aud + "|".join(parts).encode("ascii")
    return _hmac.new(_hmac_secret, body, hashlib.sha256).digest()


# Same character class as Signet's name sanitiser (src/lib/url-auth.ts).
# Letters/digits/marks plus a few punctuation marks; control chars stripped.
_HANDLE_MAX_LEN = 64
_HANDLE_DROP_RE = re.compile(r"[\x00-\x1f\x7f]")


def _sanitise_handle(raw: str | None) -> str:
    """Mirror Signet's `display_name` sanitisation: strip control chars, cap at 64."""
    if not raw:
        return ""
    cleaned = _HANDLE_DROP_RE.sub("", raw).strip()
    return cleaned[:_HANDLE_MAX_LEN]


def mint_session_cookie(
    pubkey: str,
    from_np: bool = False,
    handle: str = "",
    expires_ts: int | None = None,
) -> tuple[str, int]:
    """Return (cookie_value, expires_ts).

    Value format: `<b64url(hmac)>.<pubkey>.<expires_ts>.<np_flag>.<b64url(handle)>`
    where np_flag is "1" or "0" and handle is the persona display-name from Signet
    (UTF-8 → b64url, may be empty string when not shared). HMAC covers all four
    payload fields — none of the trailing bytes are forgeable.

    Cookies minted before the handle field was added (4 segments instead of 5)
    are still accepted by `verify_session_cookie` for the duration of their TTL
    so existing sign-ins don't get bumped.

    Note on `from_np`: signet-login v0.7.1 doesn't surface the natural-person
    fallback flag, so callers currently always pass `from_np=False`. Until
    the signet-login maintainer's upstream exposes it (see MESSAGE-FROM-AXENSTAX §0 2026-05-20),
    the cookie always carries `np_flag=0`. The HMAC still covers the field so
    we don't have to change the format when it's surfaced later.
    """
    assert _hmac_secret is not None, "auth.configure() must be called at startup"
    if expires_ts is None:
        expires_ts = int(time.time()) + COOKIE_TTL
    np_flag = "1" if from_np else "0"
    handle_clean = _sanitise_handle(handle)
    handle_b64 = _b64url(handle_clean.encode("utf-8")) if handle_clean else ""
    mac = _hmac_mac(_AUD_COOKIE, pubkey, expires_ts, np_flag, handle_b64)
    return f"{_b64url(mac)}.{pubkey}.{expires_ts}.{np_flag}.{handle_b64}", expires_ts


def verify_session_cookie(value: str) -> tuple[str, bool, str] | None:
    """Return (pubkey, from_np, handle) if the cookie HMAC verifies and hasn't expired, else None.

    Accepts both the 5-segment format (with handle) and the legacy 4-segment
    format (no handle field) — the latter for cookies minted before this
    field was added. Legacy cookies return handle="".
    """
    if _hmac_secret is None:
        return None
    if not value:
        return None
    parts = value.split(".")
    if len(parts) == 4:
        mac_b64, pubkey, exp_str, np_flag = parts
        handle_b64 = None
    elif len(parts) == 5:
        mac_b64, pubkey, exp_str, np_flag, handle_b64 = parts
    else:
        return None
    if len(pubkey) != 64 or not all(c in "0123456789abcdef" for c in pubkey):
        return None
    if np_flag not in ("0", "1"):
        return None
    try:
        expires_ts = int(exp_str)
    except ValueError:
        return None
    if expires_ts < time.time():
        return None
    try:
        expected = _hmac_mac(_AUD_COOKIE, pubkey, expires_ts, np_flag, handle_b64)
        provided = _b64url_decode(mac_b64)
    except Exception:
        return None
    if not _hmac.compare_digest(expected, provided):
        return None
    handle = ""
    if handle_b64:
        try:
            handle = _b64url_decode(handle_b64).decode("utf-8")
        except Exception:
            return None
    return pubkey, (np_flag == "1"), handle


def _cleanup_expired_challenges():
    now = int(time.time())
    expired = [c for c, exp in _pending_challenges.items() if exp < now]
    for c in expired:
        del _pending_challenges[c]


@router.post("/challenge")
async def create_challenge():
    """Issue a server-bound 64-hex challenge for signet-login to sign over.

    Returns `{challenge, expires_at}`. The client passes `challenge` into
    `Signet.login({ challenge })` so the kind-21236 event we eventually verify
    has a `challenge` tag we issued. Without this binding, an attacker who
    intercepted a signed event could replay it; with it, replay requires a
    valid unspent server-issued challenge.
    """
    async with _pending_lock:
        _cleanup_expired_challenges()
        if len(_pending_challenges) >= MAX_PENDING_CHALLENGES:
            raise HTTPException(429, "Too many pending sign-ins — try again in a moment")
        challenge = secrets.token_hex(32)
        expires_at = int(time.time()) + CHALLENGE_TTL
        _pending_challenges[challenge] = expires_at
    log.info(f"Auth challenge issued: {challenge[:8]}... expires_at={expires_at}")
    return {"challenge": challenge, "expires_at": expires_at}


class VerifyBody(BaseModel):
    auth_event: dict = Field(alias="authEvent")
    display_name: str | None = Field(default=None, max_length=128, alias="displayName")

    class Config:
        populate_by_name = True


@router.post("/verify")
async def verify(
    request: Request,
    body: VerifyBody,
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    """Verify a signet-login-delivered kind-21236 auth event and mint the session cookie.

    `signet-login` resolves `Signet.login()` (or `Signet.handleRedirectCallback()`)
    with a `SignetSession` containing a signed kind-21236 `authEvent` whose tags
    include `challenge`, `origin`, and `app`. The client POSTs that event here.
    The server:

      1. Validates `X-Requested-With: fetch` (CSRF defence)
      2. Verifies the event (schnorr sig + ID hash + challenge + origin + app + freshness)
      3. Spends the server-issued challenge so it can't be reused
      4. Sanitises the display_name (if shared)
      5. Mints the axenstax_session cookie

    Same trust level as the old `/auth/relay-complete` and `/auth/callback`
    paths, but with one endpoint instead of three.
    """
    if x_requested_with != "fetch":
        raise HTTPException(status_code=400, detail="missing X-Requested-With header")

    auth_event = body.auth_event
    if not isinstance(auth_event, dict):
        raise HTTPException(status_code=400, detail="authEvent must be an object")

    # Tags arrive verbatim from signet-login's session.authEvent.
    pubkey, error = verify_signet_auth_event(
        auth_event,
        expected_challenge="",  # filled in once we extract the challenge tag below
        expected_origin=_request_origin(request),
        expected_app=AUTH_EVENT_APP_NAME,
    )
    # Re-run verification once we know which challenge the client tried to use.
    # Why two passes? We need to peek at the challenge tag to look it up in
    # `_pending_challenges` (which is what makes this server-bound), but the
    # full verification has to *check* that the tag matches an expected value.
    # First pass discovers the tag, second pass enforces it.
    if pubkey is None and error != "challenge-mismatch":
        # Any non-challenge failure is a hard rejection — bad sig, bad shape,
        # stale event, wrong origin/app. Log and return.
        log.warning(
            f"verify rejected: {error} pubkey={(auth_event.get('pubkey', '') or '')[:16]}..."
        )
        raise HTTPException(status_code=403, detail=f"auth-event-invalid:{error}")

    # Find the challenge tag the client signed over.
    claimed_challenge = ""
    for tag in auth_event.get("tags", []):
        if isinstance(tag, list) and len(tag) >= 2 and tag[0] == "challenge" and isinstance(tag[1], str):
            claimed_challenge = tag[1].lower()
            break
    if not claimed_challenge or not _is_hex(claimed_challenge, 64):
        raise HTTPException(status_code=403, detail="auth-event-invalid:challenge-missing")

    # The challenge must be one we issued and have not yet spent.
    async with _pending_lock:
        _cleanup_expired_challenges()
        if claimed_challenge not in _pending_challenges:
            log.warning(f"verify rejected: unknown-challenge {claimed_challenge[:8]}...")
            raise HTTPException(status_code=403, detail="auth-event-invalid:unknown-challenge")
        # Spend the challenge — single-use, regardless of verification outcome.
        del _pending_challenges[claimed_challenge]

    # Now run the full verification with the known-expected challenge.
    pubkey, error = verify_signet_auth_event(
        auth_event,
        expected_challenge=claimed_challenge,
        expected_origin=_request_origin(request),
        expected_app=AUTH_EVENT_APP_NAME,
    )
    if pubkey is None:
        log.warning(
            f"verify rejected: {error} pubkey={(auth_event.get('pubkey', '') or '')[:16]}..."
        )
        raise HTTPException(status_code=403, detail=f"auth-event-invalid:{error}")

    handle_clean = _sanitise_handle(body.display_name)
    if handle_clean:
        log.info(f"verify OK: pubkey={pubkey[:16]}... handle={handle_clean!r}")
    else:
        log.info(f"verify OK: pubkey={pubkey[:16]}...")

    # `from_np` is not surfaced by signet-login v0.7.1 — see the signet-login upstream
    # ask in MESSAGE-FROM-AXENSTAX.md §0 2026-05-20. Cookie HMAC still covers
    # the field; we always pass False until the SDK exposes it.
    cookie_value, cookie_exp = mint_session_cookie(
        pubkey, from_np=False, handle=handle_clean,
    )
    # `expires_at` lets auth.js persist a JS-readable session marker
    # (`axenstax_session_until`) — Spec 32 Phase A. The HttpOnly cookie stays
    # the authoritative source; the marker is a UX hint for the offline path.
    response = JSONResponse({"ok": True, "pubkey": pubkey, "expires_at": cookie_exp})
    response.set_cookie(
        key=COOKIE_NAME,
        value=cookie_value,
        max_age=COOKIE_TTL,
        httponly=True,
        secure=_request_is_https(request),
        samesite="strict",
        path="/",
    )
    return response


@router.get("/whoami")
async def whoami(request: Request):
    """Return the pubkey the server trusts for this browser, or 401.

    Used by auth.js's cached-pubkey fast path: before booting WASM with a
    pubkey read from localStorage, the client GETs here to verify the
    session cookie actually matches the cached value. Prevents a stale or
    planted localStorage entry from booting WASM with an attacker-chosen
    pubkey.
    """
    cookie = request.cookies.get(COOKIE_NAME, "")
    verified = verify_session_cookie(cookie)
    if verified is None:
        raise HTTPException(status_code=401, detail="no valid session")
    pubkey, from_np, handle = verified
    # Sliding session (Spec 32 Phase A): re-mint with a fresh COOKIE_TTL expiry
    # on each authenticated boot, so an active player never lapses. `expires_at`
    # lets auth.js persist the `axenstax_session_until` marker.
    cookie_value, cookie_exp = mint_session_cookie(
        pubkey, from_np=from_np, handle=handle,
    )
    response = JSONResponse({
        "pubkey": pubkey,
        "from_np": from_np,
        "handle": handle,
        "expires_at": cookie_exp,
    })
    response.set_cookie(
        key=COOKIE_NAME,
        value=cookie_value,
        max_age=COOKIE_TTL,
        httponly=True,
        secure=_request_is_https(request),
        samesite="strict",
        path="/",
    )
    return response


@router.post("/logout")
async def logout(
    request: Request,
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    """Clear the session cookie. Idempotent — succeeds whether a cookie was
    present or not.

    CSRF defence: requires `X-Requested-With: fetch` so a cross-origin form
    post can't sign a user out (annoying but real). Same posture as
    /auth/verify.

    The localStorage cached pubkey lives client-side and is the JS caller's
    responsibility to clear; auth.js's `axenstax_signout` helper does both.
    """
    if x_requested_with != "fetch":
        raise HTTPException(status_code=400, detail="missing X-Requested-With header")
    response = JSONResponse({"ok": True})
    # Match the Set-Cookie attributes used at mint time (path, samesite,
    # secure on HTTPS) so browsers actually overwrite the existing cookie
    # rather than leaving it because of an attribute mismatch.
    response.set_cookie(
        key=COOKIE_NAME,
        value="",
        max_age=0,
        expires=0,
        httponly=True,
        secure=_request_is_https(request),
        samesite="strict",
        path="/",
    )
    return response
