// Tally Test Board verdicts from the feedback ledger into the per-feature counts
// the game site serves (`{ ref: { good, broken } }`). Pure + unit-tested.
//
// A verdict is a ledger row with `type === 'test-verdict'` whose `body` is JSON
// `{ ref, verdict: 'good'|'broken', note, build }` (see engine `test_board.rs`).
// We count DISTINCT testers (by `fromNpub`): a tester's LATEST verdict per
// feature wins, so they can flip broken→good after a fix. Optionally scope to a
// single build id (`opts.build`) for per-deploy auto-clear.
//
// The output is counts only — notes (possible kid PII) and npubs never appear in
// the served file. The makers still see the note + npub in the LOCAL ledger.

/**
 * @param {Array<object>} records  ledger rows (from Ledger.all())
 * @param {{build?: string}} [opts]  when `build` is set, only that build's verdicts count
 * @returns {Object<string, {good:number, broken:number}>}
 */
export function tallyVerdicts(records, opts = {}) {
  const buildFilter = opts && opts.build ? opts.build : null;

  // ref -> (npub -> { verdict, ts })  — keep only the latest verdict per (ref, npub).
  const latest = new Map();

  for (const r of records || []) {
    if (!r || r.type !== 'test-verdict') continue;
    let v;
    try {
      v = JSON.parse(r.body);
    } catch {
      continue; // malformed body — skip, never throw
    }
    if (!v || typeof v.ref !== 'string') continue;
    const verdict = v.verdict === 'broken' ? 'broken' : v.verdict === 'good' ? 'good' : null;
    if (!verdict) continue;
    if (buildFilter && v.build !== buildFilter) continue;

    const npub = r.fromNpub;
    if (!npub) continue;
    const ts = typeof r.ts === 'number' ? r.ts : 0;

    let byNpub = latest.get(v.ref);
    if (!byNpub) {
      byNpub = new Map();
      latest.set(v.ref, byNpub);
    }
    const prev = byNpub.get(npub);
    if (!prev || ts >= prev.ts) byNpub.set(npub, { verdict, ts });
  }

  const out = {};
  for (const [ref, byNpub] of latest) {
    let good = 0;
    let broken = 0;
    for (const { verdict } of byNpub.values()) {
      if (verdict === 'good') good++;
      else if (verdict === 'broken') broken++;
    }
    out[ref] = { good, broken };
  }
  return out;
}
