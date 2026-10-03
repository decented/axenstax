"""We do not log IP addresses (privacy page, 2026-10-01 owner decision).

uvicorn's access log writes the client address on every request, so every site
app must start uvicorn with access_log=False. This is a source-level check
across all of tools/sites/*/app.py so a new site cannot forget it.

Run: cd tools/sites/game && .venv/bin/python -m pytest test_no_access_log.py -q
"""

import re
from pathlib import Path

SITES = Path(__file__).resolve().parent.parent


def test_every_site_disables_uvicorn_access_log():
    apps = sorted(SITES.glob("*/app.py"))
    assert len(apps) >= 8, f"expected the site apps, found {[a.parent.name for a in apps]}"
    for app in apps:
        src = app.read_text()
        for m in re.finditer(r"uvicorn\.run\((.*?)\n?\s*\)\s*$", src, re.DOTALL | re.MULTILINE):
            assert "access_log=False" in m.group(1), f"{app.parent.name}/app.py: uvicorn.run without access_log=False"
        assert "uvicorn.run(" in src, f"{app.parent.name}/app.py has no uvicorn.run"
