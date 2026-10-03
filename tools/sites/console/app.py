#!/usr/bin/env python3
"""Axe'n'Stax — Operator Console (web sidecar, port 8101).

The web management surface for a dedicated server operator. Resolves Operator
Console amendment B-0: the engine has no HTTP server, so this is a separate
FastAPI sidecar that manages the exact on-disk policy files the engine reloads
every ~5s (see `identity.py`). Zero engine change.

Auth: the operator signs in with their **own npub** via signet-login (the same
flow the game site uses). Only the npub the server's attestation chains to may
manage the server — every other signed-in identity is refused.

Lives on the dedicated server's domain at `/admin` (Caddy reverse-proxies to
this service). See README.md + docs/operators/operator-console.md.
"""

import hashlib
import json
import logging
import os
import time
from pathlib import Path

from fastapi import FastAPI, File, Form, Header, HTTPException, Request, UploadFile
from fastapi.responses import HTMLResponse, JSONResponse, RedirectResponse, Response
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates

import identity
import studio
import wizard
import worlds
from auth import (
    COOKIE_NAME,
    configure as configure_auth,
    router as auth_router,
    verify_publish_authorization,
    verify_session_cookie,
)

logging.basicConfig(level=logging.INFO)
log = logging.getLogger("console")

BASE_DIR = Path(__file__).parent
PORT = int(os.environ.get("PORT", "8101"))
# Public path this app is mounted under, when behind a reverse proxy that strips
# the prefix (Caddy `handle_path /admin/*`). "" = served at root (local dev). The
# templates set <base href> to this so all relative URLs resolve correctly, and
# server-side redirects are prefixed with it.
BASE = os.environ.get("CONSOLE_BASE_PATH", "").rstrip("/")
BASE_HREF = (BASE + "/") if BASE else "/"
# Where the HMAC session secret + challenge state live (writable; defaults under
# the identity dir so it persists with the server's data).
DATA_DIR = Path(os.environ.get("CONSOLE_DATA_DIR", str(identity.identity_dir() / "console-web")))

app = FastAPI(title="Axe'n'Stax Operator Console")
app.mount("/static", StaticFiles(directory=str(BASE_DIR / "static")), name="static")
templates = Jinja2Templates(directory=str(BASE_DIR / "templates"))

configure_auth(templates, DATA_DIR)
app.include_router(auth_router)


# ─────────────────────────────── operator gate ───────────────────────────────


def current_operator_npub(request: Request) -> str | None:
    """The operator npub IF this browser holds a valid session for an npub allowed
    as an operator — the attestation's PRIMARY operator OR the additional-operators
    allowlist (`operators.txt`). Returns the signed-in operator's own npub, else
    None. This is THE gate."""
    ops = identity.operator_pubkeys_hex()
    if not ops:
        return None  # server not provisioned — nobody is an operator yet
    verified = verify_session_cookie(request.cookies.get(COOKIE_NAME, ""))
    if verified is None:
        return None
    pubkey, _from_np, _handle = verified
    if pubkey.lower() not in ops:
        return None  # signed in, but not an operator of THIS server
    return identity.npub_encode(pubkey.lower())


def _require_operator(request: Request) -> str:
    npub = current_operator_npub(request)
    if npub is None:
        raise HTTPException(status_code=403, detail="not-operator")
    return npub


def _session_pubkey(request: Request) -> str | None:
    """The signed-in pubkey (hex) if the session cookie is valid, else None."""
    verified = verify_session_cookie(request.cookies.get(COOKIE_NAME, ""))
    return verified[0].lower() if verified else None


def _require_cap(request: Request, cap: str) -> str:
    """Require the signed-in operator to hold capability `cap`; return their pubkey
    hex. 403 if not signed in / not an operator / lacking the capability. This is
    the role gate behind every mutating endpoint."""
    pk = _session_pubkey(request)
    if pk is None or pk not in identity.operator_pubkeys_hex():
        raise HTTPException(status_code=403, detail="not-operator")
    if cap not in identity.caps_for(pk):
        raise HTTPException(status_code=403, detail=f"missing-capability:{cap}")
    return pk


def _require_fetch(x_requested_with: str | None) -> None:
    # CSRF defence — same posture as the game site's auth endpoints.
    if x_requested_with != "fetch":
        raise HTTPException(status_code=400, detail="missing X-Requested-With header")


# ─────────────────────────────── pages ───────────────────────────────


@app.get("/", response_class=HTMLResponse)
async def index(request: Request):
    summary = identity.identity_summary()
    if not summary.get("provisioned"):
        return templates.TemplateResponse(
            request=request,
            name="login.html",
            context={
                "summary": summary,
                "operator_npub": None,
                "unprovisioned": True,
                "base_href": BASE_HREF,
            },
        )
    if current_operator_npub(request) is not None:
        # First sign-in (no wizard record yet) → straight into the setup wizard.
        if wizard.is_first_run():
            return RedirectResponse(f"{BASE}/setup", status_code=303)
        return RedirectResponse(f"{BASE}/dashboard", status_code=303)
    return templates.TemplateResponse(
        request=request,
        name="login.html",
        context={
            "summary": summary,
            "operator_npub": summary.get("operator_npub"),
            "unprovisioned": False,
            "base_href": BASE_HREF,
        },
    )


@app.get("/setup", response_class=HTMLResponse)
async def setup(request: Request):
    """The first-run (and re-run) server setup wizard. Operator-gated; needs the
    `settings` capability (owner/admin) — moderators can't reconfigure the server."""
    pk = _session_pubkey(request)
    if pk is None or pk not in identity.operator_pubkeys_hex():
        return RedirectResponse(BASE_HREF, status_code=303)
    if "settings" not in identity.caps_for(pk):
        # A moderator landed here (shouldn't, but be safe) — send them to the dashboard.
        return RedirectResponse(f"{BASE}/dashboard", status_code=303)
    return templates.TemplateResponse(
        request=request,
        name="setup.html",
        context={
            "base_href": BASE_HREF,
            "form": wizard.current_state_for_form(),
        },
    )


@app.get("/dashboard", response_class=HTMLResponse)
async def dashboard(request: Request):
    pk = _session_pubkey(request)
    if pk is None or pk not in identity.operator_pubkeys_hex():
        return RedirectResponse(BASE_HREF, status_code=303)
    caps = identity.caps_for(pk)
    summary = identity.identity_summary()
    return templates.TemplateResponse(
        request=request,
        name="dashboard.html",
        context={
            "base_href": BASE_HREF,
            "summary": summary,
            # who's looking + what they can do (template hides absent caps).
            "you_npub": identity.npub_encode(pk),
            "you_role": identity.role_of(pk),
            "caps": sorted(caps),
            "allowlist": identity.allowlist(),
            "blocklist": identity.blocklist(),
            "require_signin": identity.require_signin(),
            "settings": identity.settings(),
            "telemetry": identity.telemetry(),
            # Team panel: editable role members per role + the immutable primary owner.
            "team": {r: identity.role_members(r) for r in identity.ROLE_FILES},
            "primary_owner_npub": summary.get("operator_npub"),
            # Studio / Gallery panel: the world's exhibit images on the volume.
            "studio_world": studio.world_name(),
            "studio_images": studio.list_images() if "studio" in caps else [],
            "studio_exhibits": studio.list_exhibits() if "studio" in caps else [],
            # World / Showcase panel.
            "world_status": studio.world_status() if "showcase" in caps else None,
            # Setup wizard: show a "finish setup" banner until it's completed, and a
            # findable "re-run setup" control (owner/admin only).
            "can_setup": "settings" in caps,
            "setup_complete": wizard.setup_complete(),
            # Use-case guide: the dashboard adapts to what kind of place this is, so
            # the operator can see at a glance who can do what.
            "server_type": wizard.server_type(),
            "is_gallery": wizard.is_gallery(),
            "visitors_open": studio.showcase_config().get("enabled", False) if "showcase" in caps else False,
        },
    )


# ─────────────────────────────── setup wizard (owner/admin) ───────────────────────────────


@app.post("/api/setup/apply")
async def setup_apply(
    request: Request,
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    """Persist a completed wizard. Body is JSON: server_type, server_name, about,
    access, announce, keep_history, friends[], start_fresh_world."""
    _require_fetch(x_requested_with)
    pk = _require_cap(request, "settings")
    try:
        payload = await request.json()
    except Exception:
        raise HTTPException(400, "expected JSON body")
    if not isinstance(payload, dict):
        raise HTTPException(400, "expected a JSON object")
    # Stamp the operator who owns this world's content (recorded in the manifest).
    payload.setdefault("content_owner_npub", identity.npub_encode(pk))
    result = wizard.apply(payload)
    return JSONResponse(result)


@app.post("/api/world/create")
async def world_create(
    request: Request,
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    """Initial publish — "Add a world". Generalises the setup wizard: name + kind
    (normal/gallery) + terrain (normal / flat+ground) + access. The engine
    (re)generates and serves that world; it's recorded in the manifest with the
    operator as content owner. Build spec §4 (POST /api/world/create)."""
    _require_fetch(x_requested_with)
    pk = _require_cap(request, "settings")
    try:
        payload = await request.json()
    except Exception:
        raise HTTPException(400, "expected JSON body")
    if not isinstance(payload, dict):
        raise HTTPException(400, "expected a JSON object")
    payload.setdefault("content_owner_npub", identity.npub_encode(pk))
    result = wizard.apply(payload)
    result["worlds"] = worlds.list_worlds()
    return JSONResponse(result)


@app.get("/api/worlds")
async def list_served_worlds(request: Request):
    """List the served worlds in the manifest, for the console's worlds panel.
    Build spec §4 (GET /api/worlds)."""
    _require_operator(request)
    return JSONResponse({"ok": True, "worlds": worlds.list_worlds()})


@app.get("/api/world/{world_id}/export")
async def world_export(request: Request, world_id: str):
    """Pull to edit — stream the served world packed as `.axeworld` so the operator
    can download, edit locally, and re-publish. The console packs the on-disk world
    folder at rest (engine stays HTTP-free). Build spec §4 (GET /api/world/<id>/export).

    Auth: `settings`-capable operator cookie (owner/admin) — export pulls the
    full world content, a content operation moderators don't hold, matching the
    publish gate. The signed-event variant (§3) lands with P3's game-driven
    publish path."""
    _require_cap(request, "settings")
    if not worlds.is_valid_world_id(world_id):
        raise HTTPException(400, "bad world id")
    data = worlds.pack_world_dir(world_id)
    if data is None:
        raise HTTPException(404, "no such world on this server")
    return Response(
        content=data,
        media_type="application/octet-stream",
        headers={"Content-Disposition": f'attachment; filename="{world_id}.axeworld"'},
    )


@app.post("/api/world/{world_id}/publish")
async def world_publish(
    request: Request,
    world_id: str,
    archive: UploadFile = File(...),
    authz: str = Form(...),
):
    """Update publish — the core publish-from-game path (build spec §3, §4).

    The game uploads the packed `.axeworld` + an operator-signed authorization
    event (NOT a cookie — the game isn't the console browser, so it carries its
    own proof). The console verifies the event (sig + operator-membership + world
    + archive-hash + freshness), unpacks the world into the slot, records it, and
    drops the restart sentinel so the engine reloads onto the new content."""
    if not worlds.is_valid_world_id(world_id):
        raise HTTPException(400, "bad world id")
    # Early reject an honestly-declared oversized body before buffering it. A
    # liar can omit/forge Content-Length, so the post-read check below is the real
    # guard; for a hard cap on the bytes arriving, also set a request_body limit in
    # Caddy (the reverse proxy) — see docs/operators.
    declared = request.headers.get("content-length")
    if declared and declared.isdigit() and int(declared) > worlds.MAX_PUBLISH_UPLOAD_BYTES:
        raise HTTPException(413, "archive too large")
    data = await archive.read()
    if not data:
        raise HTTPException(400, "empty archive")
    if len(data) > worlds.MAX_PUBLISH_UPLOAD_BYTES:
        raise HTTPException(413, "archive too large")
    archive_sha = hashlib.sha256(data).hexdigest()

    try:
        event = json.loads(authz)
    except Exception:
        raise HTTPException(400, "authz must be a JSON event")

    # Trusted-signer set = holders of the `settings` capability (owner/admin),
    # NOT every operator: a moderator holds only `access`, and the console's own
    # content routes (`world_create`, `setup_apply`) gate on `settings`. Publish
    # replaces world content, so it must match — otherwise a moderator key could
    # sign an authz for something the web console forbids them.
    operator, err = verify_publish_authorization(
        event, world_id, archive_sha, identity.pubkeys_with_cap("settings")
    )
    if err:
        # One 403 for every authz failure — don't leak which check failed beyond
        # a short token (mirrors auth.py's verify error tokens).
        raise HTTPException(403, f"publish-unauthorized:{err}")

    ok, install_err = worlds.install_published_world(world_id, data)
    if not ok:
        raise HTTPException(400, f"install-failed:{install_err}")

    # Record the publish on the manifest entry + cycle the engine onto it.
    entry = worlds.get_world(world_id) or {"id": world_id}
    entry["last_published_unix"] = int(time.time())
    entry["last_published_by"] = identity.npub_encode(operator)
    entry["last_published_hash"] = archive_sha
    worlds.record_world(entry)
    wizard.request_restart()

    log.info("Published world '%s' by operator %s", world_id, identity.npub_encode(operator))
    return JSONResponse({
        "ok": True,
        "world_id": world_id,
        "operator": identity.npub_encode(operator),
        "archive_sha256": archive_sha,
        "restart": True,
    })


@app.post("/api/setup/skip")
async def setup_skip(
    request: Request,
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    """The small-text 'skip for now' link — boot on defaults, keep nudging."""
    _require_fetch(x_requested_with)
    _require_cap(request, "settings")
    return JSONResponse(wizard.skip())


@app.post("/api/setup/reset")
async def setup_reset(
    request: Request,
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    """Re-run setup: flip the console record so `/` routes back to the wizard. The
    engine keeps running on its current config until the re-run is applied."""
    _require_fetch(x_requested_with)
    _require_cap(request, "settings")
    return JSONResponse(wizard.reset())


@app.post("/api/gallery/visitors")
async def gallery_visitors(
    request: Request,
    open: str = Form(...),
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    """The gallery 'Build mode ⇄ Open to visitors' switch. Open = arm the web kiosk
    (showcase.json) so anyone opening the link is read-only; Build = disarm it so the
    operator can edit. Live (applies to a visitor the next time they open the link) —
    no world change, no restart."""
    _require_fetch(x_requested_with)
    _require_cap(request, "showcase")
    on = open.lower() in ("1", "true", "yes", "on")
    cfg = studio.set_showcase_config(on, "board", 0)
    return JSONResponse({"ok": True, "visitors_open": cfg.get("enabled", False)})


# ─────────────────────────────── policy API (operator-gated) ───────────────────────────────


@app.post("/api/list/{which}/{action}")
async def list_edit(
    which: str,
    action: str,
    request: Request,
    npub: str = Form(...),
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    _require_fetch(x_requested_with)
    _require_cap(request, "access")
    if which not in ("allow", "block") or action not in ("add", "remove"):
        raise HTTPException(400, "bad request")
    try:
        if action == "add":
            identity.add_to_list(which, npub)
        else:
            identity.remove_from_list(which, npub)
    except ValueError as e:
        raise HTTPException(400, str(e))
    return JSONResponse({"ok": True, "allowlist": identity.allowlist(), "blocklist": identity.blocklist()})


@app.post("/api/require-signin")
async def require_signin_set(
    request: Request,
    on: str = Form(...),
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    _require_fetch(x_requested_with)
    _require_cap(request, "require_signin")
    identity.set_require_signin(on.lower() in ("1", "true", "yes", "on"))
    return JSONResponse({"ok": True, "require_signin": identity.require_signin()})


@app.post("/api/settings")
async def settings_set(
    request: Request,
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    _require_fetch(x_requested_with)
    _require_cap(request, "settings")
    form = await request.form()
    updates: dict = {}
    if "server_name" in form:
        v = str(form["server_name"]).strip()
        updates["server_name"] = v or None
    for key in ("about", "region", "privacy_level"):
        if key in form:
            updates[key] = str(form[key]).strip()
    if "announce" in form:
        updates["announce"] = str(form["announce"]).lower() in ("1", "true", "yes", "on")
    if "max_players" in form:
        raw = str(form["max_players"]).strip()
        updates["max_players"] = int(raw) if raw.isdigit() else None
    if "privacy_retention_days" in form:
        raw = str(form["privacy_retention_days"]).strip()
        updates["privacy_retention_days"] = int(raw) if raw.isdigit() else 0
    saved = identity.save_settings(updates)
    return JSONResponse({"ok": True, "settings": saved})


@app.post("/api/kick")
async def kick(
    request: Request,
    npub: str = Form(...),
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    _require_fetch(x_requested_with)
    _require_cap(request, "access")
    try:
        identity.queue_kick(npub)
    except ValueError as e:
        raise HTTPException(400, str(e))
    return JSONResponse({"ok": True})


@app.post("/api/forget")
async def forget(
    request: Request,
    npub: str = Form(...),
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    _require_fetch(x_requested_with)
    _require_cap(request, "privacy")
    identity.forget_player(npub)
    return JSONResponse({"ok": True, "telemetry": identity.telemetry()})


@app.post("/api/purge-history")
async def purge_history(
    request: Request,
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    _require_fetch(x_requested_with)
    _require_cap(request, "privacy")
    identity.purge_history()
    return JSONResponse({"ok": True, "telemetry": identity.telemetry()})


# ─────────────────────────────── team / roles (owner + admin) ───────────────────────────────


@app.post("/api/team/{role}/{action}")
async def team_edit(
    role: str,
    action: str,
    request: Request,
    npub: str = Form(...),
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    _require_fetch(x_requested_with)
    if role not in identity.ROLE_FILES or action not in ("add", "remove"):
        raise HTTPException(400, "bad request")
    # You need the matching manage_<role>s capability (owner: all; admin: moderators).
    _require_cap(request, f"manage_{role}s")
    try:
        if action == "add":
            identity.add_role_member(role, npub)
        else:
            identity.remove_role_member(role, npub)
    except ValueError as e:
        raise HTTPException(400, str(e))
    return JSONResponse(
        {"ok": True, "team": {r: identity.role_members(r) for r in identity.ROLE_FILES}}
    )


# ─────────────────────────────── studio / gallery (admin+) ───────────────────────────────


@app.get("/api/studio/images")
async def studio_images(request: Request):
    _require_cap(request, "studio")
    return JSONResponse(
        {"ok": True, "world": studio.world_name(), "images": studio.list_images()}
    )


@app.post("/api/studio/upload")
async def studio_upload(
    request: Request,
    file: UploadFile = File(...),
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    _require_fetch(x_requested_with)
    _require_cap(request, "studio")
    data = await file.read()
    try:
        name = studio.save_image(file.filename or "upload.png", data)
    except ValueError as e:
        raise HTTPException(400, str(e))
    return JSONResponse({"ok": True, "name": name, "images": studio.list_images()})


@app.post("/api/studio/delete")
async def studio_delete(
    request: Request,
    name: str = Form(...),
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    _require_fetch(x_requested_with)
    _require_cap(request, "studio")
    try:
        studio.delete_image(name)
    except ValueError as e:
        raise HTTPException(400, str(e))
    return JSONResponse({"ok": True, "images": studio.list_images()})


# ─────────────────────────────── world / showcase (admin+) ───────────────────────────────


@app.post("/api/showcase")
async def showcase_set(
    request: Request,
    enabled: str = Form(...),
    exit_action: str = Form("board"),
    auto_loop_secs: str = Form("0"),
    x_requested_with: str | None = Header(default=None, alias="X-Requested-With"),
):
    _require_fetch(x_requested_with)
    _require_cap(request, "showcase")
    cfg = studio.set_showcase_config(
        enabled.lower() in ("1", "true", "yes", "on"), exit_action, auto_loop_secs
    )
    return JSONResponse({"ok": True, "showcase": cfg})


@app.get("/healthz")
async def healthz():
    return {"ok": True, "provisioned": bool(identity.operator_pubkeys_hex())}


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host="0.0.0.0", port=PORT, access_log=False)
