// Ledger privacy guarantees: 0600 file mode and 180-day retention.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { mkdtempSync, rmSync, writeFileSync, statSync, readFileSync, chmodSync } from 'node:fs';
import { Ledger, RETENTION_DAYS } from '../lib/ledger.mjs';

const DAY = 86400;
const NOW = 1_800_000_000;
const clock = { now: () => NOW };
function tmp() {
  const dir = mkdtempSync(path.join(tmpdir(), 'axe-ledger-priv-'));
  return { p: path.join(dir, 'l.jsonl'), cleanup: () => rmSync(dir, { recursive: true, force: true }) };
}
const mode = (p) => statSync(p).mode & 0o777;
const row = (id, ts) => ({ id, ts, type: 'bug', body: 'x', status: 'new' });

test('writes are mode 0600', () => {
  const { p, cleanup } = tmp();
  try {
    const l = new Ledger(p, clock);
    l.append(row('a', NOW));
    assert.equal(mode(p), 0o600);
    l.update('a', { status: 'triaged' });
    assert.equal(mode(p), 0o600);
  } finally { cleanup(); }
});

test('an existing world-readable file is chmodded to 0600 on open', () => {
  const { p, cleanup } = tmp();
  try {
    writeFileSync(p, JSON.stringify(row('a', NOW)) + '\n', { mode: 0o664 });
    chmodSync(p, 0o664);
    new Ledger(p, clock);
    assert.equal(mode(p), 0o600);
  } finally { cleanup(); }
});

test('rows older than the retention window are pruned on load and persisted', () => {
  const { p, cleanup } = tmp();
  try {
    const old = row('old', NOW - (RETENTION_DAYS + 1) * DAY);
    const edge = row('edge', NOW - (RETENTION_DAYS - 1) * DAY);
    const undated = { id: 'undated', ts: null, type: 'bug', body: 'x', status: 'new' };
    writeFileSync(p, [old, edge, undated].map((r) => JSON.stringify(r)).join('\n') + '\n');
    const l = new Ledger(p, clock);
    assert.deepEqual(l.all().map((r) => r.id).sort(), ['edge', 'undated']);
    assert.ok(!readFileSync(p, 'utf8').includes('"old"'), 'pruned row must be gone from disk');
  } finally { cleanup(); }
});

test('a stale report is never stored; rows that age out while running are pruned on write', () => {
  const { p, cleanup } = tmp();
  try {
    let t = NOW;
    const l = new Ledger(p, { now: () => t });
    assert.equal(l.append(row('stale', NOW - (RETENTION_DAYS + 5) * DAY)), false);
    assert.equal(l.has('stale'), false);
    l.append(row('a', NOW));
    t = NOW + (RETENTION_DAYS + 1) * DAY;
    l.append(row('b', t));
    assert.deepEqual(l.all().map((r) => r.id), ['b']);
  } finally { cleanup(); }
});
