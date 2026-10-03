// Emit the served Test Board status file from the LOCAL feedback ledger.
//
//   node test-board.mjs [--build <id>] [--out <path>]
//
// Reuses the same ledger read.mjs ingests into, tallies the `test-verdict` rows
// (lib/test_board.mjs), and writes `{ ref: { good, broken } }` for the game site
// to serve at /api/test-board. Counts only — notes + npubs stay in the local
// ledger, never in this file (privacy: owner directive, feedback internal-only).
//
// Run it after `node read.mjs --once` (or on a timer alongside the live reader)
// to refresh the lobby's traffic-light dots. Pass `--build <crate-version>` to
// scope to a single build (per-deploy auto-clear); omit to count each tester's
// latest verdict across builds.
import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { Ledger } from './lib/ledger.mjs';
import { tallyVerdicts } from './lib/test_board.mjs';

const args = process.argv.slice(2);
function opt(name) {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
}

const LEDGER = process.env.LEDGER || path.resolve('feedback-ledger.jsonl');
const OUT =
  opt('--out') ||
  process.env.TEST_BOARD_OUT ||
  path.resolve('..', 'sites', 'game', 'data', 'test-board-status.json');
const build = opt('--build') || process.env.TEST_BOARD_BUILD || undefined;

const ledger = new Ledger(LEDGER);
const status = tallyVerdicts(ledger.all(), { build });
writeFileSync(OUT, JSON.stringify(status, null, 2) + '\n', 'utf8');

const refs = Object.keys(status);
console.log(`test-board: tallied ${refs.length} feature(s)${build ? ` for build ${build}` : ''} → ${OUT}`);
for (const ref of refs) console.log(`  ${ref}: ${status[ref].good} ✓  ${status[ref].broken} ✗`);
