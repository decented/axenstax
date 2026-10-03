"""Shared fixtures for the game-site tests.

The app reads config from env at import time, so fixtures that need a
particular config set it *before* importing app, using importlib.reload.
"""
import importlib
import os
import sys

import pytest


@pytest.fixture
def app_module(monkeypatch):
    """Import (or reload) app.py with cloud save enabled.

    Cloud save is serverless: the only server-side config is the public Blossom
    URL the client is pointed at. Setting it makes _cloud_save_enabled() true.
    """
    monkeypatch.setenv("BLOSSOM_PUBLIC_URL", "https://blossom.test")
    # The site's modules are importable because tests run from the site dir.
    sys.path.insert(0, os.path.dirname(os.path.dirname(__file__)))
    import app as app_module  # noqa: E402
    importlib.reload(app_module)
    app_module.startup()
    return app_module


@pytest.fixture
def client(app_module):
    from starlette.testclient import TestClient
    return TestClient(app_module.app)
