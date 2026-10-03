#!/usr/bin/env python3
"""Generate / inspect single-use claim codes. Run on the host (admin only).

  python gen_codes.py 50            # make 50 new codes, print them
  python gen_codes.py 50 --csv      # ...as CSV (code,claim_url) for printing cards
  python gen_codes.py --status      # totals: total / used / available

CLAIM_URL_BASE (env) sets the QR/card link base, e.g. https://claim.axenstax.com
The codes live in data/codes.json (override with CLAIM_DATA_DIR).
"""

import os
import sys

import codes


def main(argv: list[str]) -> int:
    args = [a for a in argv[1:]]
    if "--status" in args:
        s = codes.summary()
        print(f"total={s['total']}  used={s['used']}  available={s['available']}")
        return 0

    csv = "--csv" in args
    nums = [a for a in args if a.isdigit()]
    if not nums:
        print(__doc__)
        return 1
    n = int(nums[0])
    base = os.environ.get("CLAIM_URL_BASE", "https://claim.axenstax.com").rstrip("/")
    new = codes.generate(n)
    if csv:
        print("code,claim_url")
        for c in new:
            print(f"{c},{base}/?code={c}")
    else:
        for c in new:
            print(f"{c}   {base}/?code={c}")
    print(f"\n{len(new)} codes added. {codes.summary()}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
