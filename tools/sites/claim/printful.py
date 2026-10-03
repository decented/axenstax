"""Minimal Printful v1 API client — list synced variants + create/confirm orders.

Fulfilment / back-office ONLY. There is no payment or checkout here: the buyer
pays you in person (cash or sats), you hand them a single-use code, and this just
pushes their self-entered address to Printful so the item ships. Printful bills
*your* account for product + shipping; price your in-person sale to cover it.

v1 API: base https://api.printful.com/, `Authorization: Bearer <private token>`.
  - GET /store/products              -> sync products [{id, name, ...}]
  - GET /store/products/{id}         -> {sync_product, sync_variants:[{id, name, size, color, retail_price, synced}]}
  - POST /orders {recipient, items:[{sync_variant_id, quantity}]} -> draft order
  - POST /orders/{id}/confirm        -> confirm a draft for fulfilment
"""

import logging
import os
from dataclasses import dataclass

import requests

log = logging.getLogger(__name__)

API_BASE = "https://api.printful.com"
TIMEOUT = 20


def _token() -> str:
    return os.environ.get("PRINTFUL_TOKEN", "").strip()


def _store_id() -> str:
    return os.environ.get("PRINTFUL_STORE_ID", "").strip()


def configured() -> bool:
    """True if a private token is set — otherwise the service runs in a safe,
    read-only "not configured" state (form shows a notice, no orders possible)."""
    return bool(_token())


class PrintfulError(Exception):
    pass


def _headers() -> dict:
    h = {"Authorization": f"Bearer {_token()}", "Content-Type": "application/json"}
    sid = _store_id()
    if sid:
        h["X-PF-Store-Id"] = sid
    return h


def _request(method: str, path: str, *, json_body=None):
    if not configured():
        raise PrintfulError("Printful token not configured")
    try:
        r = requests.request(
            method, f"{API_BASE}{path}", headers=_headers(), json=json_body, timeout=TIMEOUT
        )
    except requests.RequestException as e:
        raise PrintfulError(f"Network error talking to Printful: {e}") from e
    try:
        data = r.json()
    except ValueError:
        raise PrintfulError(f"Printful returned non-JSON ({r.status_code})")
    if not r.ok:
        err = data.get("error") or {}
        raise PrintfulError(f"Printful {r.status_code}: {err.get('message') or r.text[:200]}")
    return data.get("result")


@dataclass
class Variant:
    sync_variant_id: int
    product_id: int
    product_name: str
    name: str
    size: str
    color: str
    price: str

    @property
    def label(self) -> str:
        bits = [self.product_name]
        extra = " / ".join(b for b in (self.color, self.size) if b)
        if extra:
            bits.append(extra)
        return " — ".join(bits)


def list_variants() -> list[Variant]:
    """Flat list of every synced variant across all sync products (N+1 calls —
    fine for a handful of products). Raises PrintfulError on API/network failure."""
    products = _request("GET", "/store/products") or []
    out: list[Variant] = []
    for p in products:
        detail = _request("GET", f"/store/products/{p['id']}") or {}
        sp = detail.get("sync_product") or {}
        pname = sp.get("name") or p.get("name") or "Product"
        for v in detail.get("sync_variants") or []:
            if v.get("synced") is False:
                continue
            out.append(
                Variant(
                    sync_variant_id=int(v["id"]),
                    product_id=int(p["id"]),
                    product_name=pname,
                    name=v.get("name") or pname,
                    size=v.get("size") or "",
                    color=v.get("color") or "",
                    price=str(v.get("retail_price") or ""),
                )
            )
    return out


def create_order(sync_variant_id: int, recipient: dict, *, confirm: bool = False) -> dict:
    """Create a single-item order for `sync_variant_id` shipping to `recipient`.

    Created as a DRAFT (Printful neither charges nor ships) unless `confirm=True`.
    Returns {"id", "status"}. Raises PrintfulError on failure (caller should then
    release the reserved code).
    """
    body = {
        "recipient": recipient,
        "items": [{"sync_variant_id": int(sync_variant_id), "quantity": 1}],
    }
    result = _request("POST", "/orders", json_body=body) or {}
    order_id = result.get("id")
    status = result.get("status")
    if confirm and order_id:
        confirmed = _request("POST", f"/orders/{order_id}/confirm", json_body={}) or {}
        status = confirmed.get("status") or status
    return {"id": order_id, "status": status}
