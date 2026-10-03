#!/usr/bin/env node
// Print the local ledger (no relay, no key — safe to run anytime). `new` reports
// first, then triaged, then resolved. Bugs/ideas live ONLY here — npub + handle +
// content — and are triaged internally (verdict bug/idea/discard); they are NEVER
// filed to GitHub (owner directive). See README.md and the spec.
//
//   LEDGER=./feedback-ledger.jsonl node list.mjs [--new]
import path from 'node:path';
import { Ledger } from './lib/ledger.mjs';

const LEDGER = process.env.LEDGER || path.resolve('feedback-ledger.jsonl');
const onlyNew = process.argv.includes('--new');

const ledger = new Ledger(LEDGER);
let rows = ledger.all();
if (onlyNew) rows = rows.filter((r) => r.status === 'new');
const rank = { new: 0, triaged: 1, resolved: 2 };
rows.sort((a, b) => (rank[a.status] ?? 9) - (rank[b.status] ?? 9) || (a.ts || 0) - (b.ts || 0));

if (!rows.length) { console.log('(ledger empty' + (onlyNew ? ' — no new reports' : '') + ')'); process.exit(0); }
for (const r of rows) {
  const verdict = r.verdict ? ` verdict=${r.verdict}` : '';
  // handle, origin (client) and personaNpub are all sender-asserted tags —
  // nip59 verifies WHO sent a report (fromNpub), never WHAT they claimed
  // about themselves. Label every one so a triager never mistakes a forged
  // claim for a proven identity (2026-09-27 audit, REVIEW-W6 should-fix #2).
  const who = r.handle ? `${r.handle} (unverified) · ${r.fromNpub.slice(0, 12)}…` : `${r.fromNpub.slice(0, 12)}…`;
  const build = r.build ? ` · v${r.build}` : '';
  const client = r.origin ? ` · client=${r.origin} (unverified)` : '';
  const persona = r.personaNpub ? ` · persona=${r.personaNpub.slice(0, 12)}… (unverified)` : '';
  console.log(`${r.status.toUpperCase().padEnd(8)} ${r.type.padEnd(5)} ${r.id}  ${who}${build}${client}${persona}${verdict}`);
  console.log(`         ${(r.body || '').replace(/\s+/g, ' ').slice(0, 100)}`);
}
console.log(`\n${rows.length} report(s); ${ledger.newReports().length} awaiting triage.`);
