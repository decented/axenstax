#!/usr/bin/env python3
"""Render a site's prod .env from its template, merged with any existing host .env.

Invoked on the routing box by remote-update.sh. Lets CI provision a brand-new
site AND refresh an existing one's cross-site URLs at cutover, without ever
destroying a secret a human set on the host.

Merge rules (safe by construction):
  - A NON-BLANK value in the template is authoritative config (ports, *_URL,
    CORS_ORIGINS, BLOSSOM_PUBLIC_URL, PRINTFUL_AUTO_CONFIRM). The template wins —
    this is what refreshes URLs on the .com/.org cutover. If you want a different
    value, change the template (it's in git), not the host file.
  - A BLANK value in the template means "fill on host, don't clobber":
      * keep the existing host value if one is set (preserves NOSTR_SERVER_KEY,
        LIGHTNING_TIP_ADDRESS, PRINTFUL_TOKEN, SOURCE_URL-once-public, …),
      * else emit blank (the safe default — e.g. PRINTFUL_TOKEN blank = "not open
        yet"; VOICE_SERVER_ORIGIN blank = widget off),
      * EXCEPT NOSTR_SERVER_KEY, which is generated fresh (32-byte hex) when
        absent so the server identity is stable instead of ephemeral-per-restart.
  - Keys in the existing host .env but ABSENT from the template are treated as
    host-only secrets and preserved verbatim.
  - Template comments / blank lines are kept for readability.

Output is deterministic given the same inputs, so re-running produces an
identical file (no spurious restarts-from-diff).

Usage: render-env.py <template> <existing-or-missing-path>   # prints to stdout
"""
import re
import secrets
import sys

KEY_RE = re.compile(r"\s*([A-Za-z_][A-Za-z0-9_]*)\s*=(.*)$")


def parse(path):
    """KEY -> value for non-comment KEY=VALUE lines; missing file = {}."""
    out = {}
    try:
        with open(path, encoding="utf-8") as fh:
            for line in fh:
                if line.lstrip().startswith("#"):
                    continue
                m = KEY_RE.match(line.rstrip("\n"))
                if m:
                    out[m.group(1)] = m.group(2).strip()
    except FileNotFoundError:
        pass
    return out


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: render-env.py <template> <existing-or-missing-path>")
    tmpl_path, existing_path = sys.argv[1], sys.argv[2]
    existing = parse(existing_path)

    lines, emitted = [], set()
    with open(tmpl_path, encoding="utf-8") as fh:
        for raw in fh:
            line = raw.rstrip("\n")
            if line.lstrip().startswith("#"):
                lines.append(line)
                continue
            m = KEY_RE.match(line)
            if not m:
                lines.append(line)
                continue
            key, tval = m.group(1), m.group(2).strip()
            if tval:
                val = tval                                  # template config wins
            elif existing.get(key):
                val = existing[key]                         # preserve host value
            elif key == "NOSTR_SERVER_KEY":
                val = secrets.token_hex(32)                 # generate once
            else:
                val = ""                                    # genuine blank default
            lines.append(f"{key}={val}")
            emitted.add(key)

    extra = [k for k in existing if k not in emitted]
    if extra:
        lines.append("")
        lines.append("# --- preserved from existing host .env (not in template) ---")
        for k in extra:
            lines.append(f"{k}={existing[k]}")

    sys.stdout.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
