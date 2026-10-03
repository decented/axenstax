// Status-board tests (spec docs/foundations/2026-10-01-feedback-status-board.md,
// S2/S3/S8). TEST keys + a fake relay; the live publish with the real key is
// the owner boundary.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { mkdtempSync, rmSync, writeFileSync, statSync, readFileSync } from 'node:fs';
import * as NT from 'nostr-tools';

import { signerFromSecretKey } from '../lib/signer.mjs';
import {
  BOARD_D, BOARD_KIND, MAX_AGE_SECS, MAX_ENTRIES,
  ticketKey, emptyBoard, loadBoard, saveBoard, setStatus, prune, boardTemplate, publishBoard, makeAutoReceiver,
} from '../lib/board.mjs';

const official = signerFromSecretKey(NT.generateSecretKey());
function fakeRelay() {
  const published = [];
  return { published, async publish(ev) { published.push(ev); } };
}
function tmpFile() {
  const dir = mkdtempSync(path.join(tmpdir(), 'axe-board-'));
  return { file: path.join(dir, 'feedback-status-board.json'), cleanup: () => rmSync(dir, { recursive: true, force: true }) };
}

test('ticketKey matches the game (same vectors as native_mailbox/board.rs)', () => {
  assert.equal(ticketKey('abc'), '5ebff49fd28c07c0eedbfb153d274842');
  assert.equal(ticketKey('00112233445566778899aabbccddeeff'), '0f90108eb8fc60ddb38e0a477df92cac');
  assert.equal(ticketKey('abc').length, 32);
});

test('setStatus records status/at (+ version only for fixed) under the scrambled key', () => {
  const b = emptyBoard();
  setStatus(b, 'tkt-1', 'fixed', { version: '0.2.28', now: 1000 });
  setStatus(b, 'tkt-2', 'wontfix', { now: 1001 });
  setStatus(b, 'tkt-3', 'received', { now: 1002 });
  assert.deepEqual(b.t[ticketKey('tkt-1')], { s: 'fixed', at: 1000, v: '0.2.28' });
  assert.deepEqual(b.t[ticketKey('tkt-2')], { s: 'wontfix', at: 1001 });
  assert.deepEqual(b.t[ticketKey('tkt-3')], { s: 'received', at: 1002 });
  assert.ok(!JSON.stringify(b).includes('tkt-1'), 'the ticket itself never appears on the board');
});

test('setStatus validates status and version; onlyIfAbsent never downgrades', () => {
  const b = emptyBoard();
  assert.throws(() => setStatus(b, 't', 'exploded'), /status must be/);
  assert.throws(() => setStatus(b, 't', 'fixed', { version: 'bad version' }), /version must be/);
  assert.throws(() => setStatus(b, 't', 'fixed', { version: 'x'.repeat(25) }), /version must be/);
  setStatus(b, 't', 'fixed', { version: '1.0.0', now: 5 });
  assert.equal(setStatus(b, 't', 'received', { now: 6, onlyIfAbsent: true }), false);
  assert.equal(b.t[ticketKey('t')].s, 'fixed', 'auto-received did not downgrade a fixed report');
  assert.equal(setStatus(b, 'new', 'received', { now: 7, onlyIfAbsent: true }), true);
});

test('prune drops entries older than 180 days, then the oldest beyond 2000', () => {
  const now = 10_000_000;
  const b = emptyBoard();
  b.t.old = { s: 'received', at: now - MAX_AGE_SECS - 1 };
  b.t.edge = { s: 'received', at: now - MAX_AGE_SECS };
  b.t.junk = { s: 'received' }; // no timestamp: dropped
  assert.equal(prune(b, now), 2);
  assert.deepEqual(Object.keys(b.t), ['edge']);

  const big = emptyBoard();
  for (let i = 0; i < MAX_ENTRIES + 25; i++) big.t[`k${i}`] = { s: 'received', at: now - 1000 + i };
  assert.equal(prune(big, now), 25);
  assert.equal(Object.keys(big.t).length, MAX_ENTRIES);
  assert.ok(!('k0' in big.t) && !('k24' in big.t), 'the 25 oldest went');
  assert.ok('k25' in big.t && `k${MAX_ENTRIES + 24}` in big.t);
});

test('boardTemplate is an addressable kind-30078 with d tag and plain-JSON content; created_at always advances', () => {
  const b = emptyBoard();
  setStatus(b, 't', 'received', { now: 100 });
  const t1 = boardTemplate(b, 100);
  assert.equal(t1.kind, BOARD_KIND);
  assert.deepEqual(t1.tags, [['d', BOARD_D]]);
  const content = JSON.parse(t1.content);
  assert.equal(content.v, 1);
  assert.equal(content.updated, 100);
  assert.deepEqual(Object.keys(content.t), [ticketKey('t')]);
  // A second publish in the same second still gets a strictly newer created_at.
  b.updated = t1.created_at;
  assert.equal(boardTemplate(b, 100).created_at, 101);
});

test('publishBoard signs with the official key, prunes, and persists only after a successful publish', async () => {
  const { file, cleanup } = tmpFile();
  try {
    const relay = fakeRelay();
    const now = 5_000_000;
    const board = emptyBoard();
    setStatus(board, 'tkt', 'fixed', { version: '0.2.28', now });
    board.t.stale = { s: 'received', at: now - MAX_AGE_SECS - 5 };

    const ev = await publishBoard({ board, file, signer: official, relay, now });
    assert.equal(relay.published.length, 1);
    assert.equal(ev.pubkey, official.pubkey);
    assert.ok(NT.verifyEvent(ev), 'valid signature');
    assert.equal(ev.kind, 30078);
    assert.deepEqual(ev.tags, [['d', 'axenstax-feedback-status']]);
    const content = JSON.parse(ev.content);
    assert.ok(content.t[ticketKey('tkt')] && !content.t.stale, 'stale entry pruned from the published board');

    // Saved, 0600, and reloads identically.
    assert.equal(statSync(file).mode & 0o777, 0o600);
    const reloaded = loadBoard(file);
    assert.deepEqual(reloaded.t, board.t);
    assert.equal(reloaded.updated, ev.created_at);

    // A failing relay leaves the file as it was.
    const before = readFileSync(file, 'utf8');
    const bad = { async publish() { throw new Error('relay down'); } };
    setStatus(board, 'tkt2', 'wontfix', { now: now + 1 });
    await assert.rejects(publishBoard({ board, file, signer: official, relay: bad, now: now + 1 }), /relay down/);
    assert.equal(readFileSync(file, 'utf8'), before, 'failed publish did not persist');
  } finally { cleanup(); }
});

test('loadBoard: missing file is empty, corrupt file is an error (never silently wiped)', () => {
  const { file, cleanup } = tmpFile();
  try {
    assert.deepEqual(loadBoard(file), emptyBoard());
    writeFileSync(file, 'not json');
    assert.throws(() => loadBoard(file));
    writeFileSync(file, JSON.stringify({ v: 2, t: {} }));
    assert.throws(() => loadBoard(file), /not a v1/);
  } finally { cleanup(); }
});

test('saveBoard/loadBoard round trip', () => {
  const { file, cleanup } = tmpFile();
  try {
    const b = emptyBoard();
    setStatus(b, 'x', 'received', { now: 9 });
    saveBoard(file, b);
    assert.deepEqual(loadBoard(file).t, b.t);
  } finally { cleanup(); }
});

test('auto-received marks new reports once, never downgrades, and publishes a single event per burst', async () => {
  const { file, cleanup } = tmpFile();
  try {
    const relay = fakeRelay();
    // A previously hand-set "fixed" must survive an auto "received" for the same id.
    const seed = emptyBoard();
    setStatus(seed, 'already-fixed', 'fixed', { version: '0.2.27', now: 1 });
    saveBoard(file, seed);

    const auto = makeAutoReceiver({ file, signer: official, relay, debounceMs: 0, now: () => 2000 });
    auto.add('new-1');
    auto.add('new-2');
    auto.add('already-fixed');
    assert.equal(await auto.flush(), 2);
    assert.equal(relay.published.length, 1, 'one publish for the burst');
    const b = loadBoard(file);
    assert.equal(b.t[ticketKey('new-1')].s, 'received');
    assert.equal(b.t[ticketKey('new-2')].s, 'received');
    assert.equal(b.t[ticketKey('already-fixed')].s, 'fixed');

    // Nothing pending ⇒ no publish.
    assert.equal(await auto.flush(), 0);
    assert.equal(relay.published.length, 1);
    // Re-adding a known id publishes nothing.
    auto.add('new-1');
    assert.equal(await auto.flush(), 0);
    assert.equal(relay.published.length, 1);
  } finally { cleanup(); }
});
