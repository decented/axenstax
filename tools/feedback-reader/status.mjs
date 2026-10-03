#!/usr/bin/env node
// Publish a report's status to the public feedback status board (OWNER-RUN,
// holds the AxeNStax secret). Replaces reply.mjs: nobody is messaged. The game
// recognises its own ticket on the board and shows "Fixed in vX" in /mailbox.
//
//   node status.mjs <report-id> received|fixed|wontfix [--version X] [--dry-run]
//
//   KEY_FILE=…   (default ~/.config/axenstax/axenstax-official.json)
//   BOARD=…      local board file (default ./feedback-status-board.json, 0600)
//   RELAYS=a,b   publish relays (default: the inbox relays the game reads)
//   LEDGER=…     only used to warn when the report id is not in the ledger
//
// Spec: docs/foundations/2026-10-01-feedback-status-board.md (S2/S3/S8).
import { homedir } from 'node:os';
import path from 'node:path';
import { loadAxenstaxSigner } from './lib/signer.mjs';
import { Ledger } from './lib/ledger.mjs';
import { makeRelay } from './lib/relay.mjs';
import {
  DEFAULT_RELAYS, STATUSES, loadBoard, publishBoard, setStatus, prune, boardTemplate, ticketKey,
} from './lib/board.mjs';

const KEY_FILE = process.env.KEY_FILE || path.join(homedir(), '.config', 'axenstax', 'axenstax-official.json');
const BOARD = process.env.BOARD || path.resolve('feedback-status-board.json');
const LEDGER = process.env.LEDGER || path.resolve('feedback-ledger.jsonl');
const RELAYS = process.env.RELAYS ? process.env.RELAYS.split(',').map((s) => s.trim()).filter(Boolean) : DEFAULT_RELAYS;

const argv = process.argv.slice(2);
const dryRun = argv.includes('--dry-run');
const vIdx = argv.indexOf('--version');
const version = vIdx >= 0 ? argv[vIdx + 1] : undefined;
const positional = argv.filter((a, i) => !a.startsWith('--') && argv[i - 1] !== '--version');
const [reportId, status] = positional;

function usage(msg) {
  if (msg) console.error(msg);
  console.error('usage: node status.mjs <report-id> received|fixed|wontfix [--version X] [--dry-run]');
  process.exit(2);
}
if (!reportId || !STATUSES.includes(status)) usage();
if (vIdx >= 0 && !version) usage('--version needs a value');
if (version && status !== 'fixed') usage('--version only applies to "fixed"');

let ledger = null;
try {
  ledger = new Ledger(LEDGER);
  if (!ledger.has(reportId)) {
    console.warn(`status: warning — ${reportId} is not in ${LEDGER}; a typo'd id publishes an entry no game will match`);
  }
} catch { /* ledger is optional here */ }

const board = loadBoard(BOARD);
setStatus(board, reportId, status, { version });
console.log(`status: ${reportId} -> ${status}${version ? ' v' + version : ''}  (board key ${ticketKey(reportId)})`);

if (dryRun) {
  prune(board);
  console.log(JSON.stringify(boardTemplate(board), null, 1));
  console.log('status: --dry-run, nothing published or saved');
  process.exit(0);
}

const signer = loadAxenstaxSigner(KEY_FILE);
const relay = makeRelay(RELAYS);
try {
  const ev = await publishBoard({ board, file: BOARD, signer, relay });
  console.log(`status: published board event ${ev.id.slice(0, 10)}… (${Object.keys(board.t).length} entries) to ${RELAYS.join(', ')}`);
  // Owner-local bookkeeping only (never published): a fixed/wontfix report is done.
  if (ledger && status !== 'received' && ledger.has(reportId)) ledger.update(reportId, { status: 'resolved' });
} finally {
  relay.close();
}
