// Test Board tally — pure verdict aggregation over ledger rows. Proves the
// distinct-tester counting, the latest-verdict-wins rule, the build filter, and
// that the published output carries only counts (no notes / npubs / PII).
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { tallyVerdicts } from '../lib/test_board.mjs';

function verdictRow(npub, ref, verdict, ts, { build = 'v1', note = '' } = {}) {
  return {
    id: `${npub}-${ref}-${ts}`,
    fromNpub: npub,
    type: 'test-verdict',
    ts,
    body: JSON.stringify({ t: 'test-verdict', ref, verdict, note, build }),
  };
}

test('counts distinct testers good/broken per ref', () => {
  const out = tallyVerdicts([
    verdictRow('npubA', 'TB-07', 'good', 10),
    verdictRow('npubB', 'TB-07', 'good', 11),
    verdictRow('npubC', 'TB-07', 'broken', 12),
    verdictRow('npubA', 'TB-08', 'broken', 13),
  ]);
  assert.deepEqual(out['TB-07'], { good: 2, broken: 1 });
  assert.deepEqual(out['TB-08'], { good: 0, broken: 1 });
});

test('latest verdict per tester wins (flip broken→good after a fix)', () => {
  const out = tallyVerdicts([
    verdictRow('npubA', 'TB-07', 'broken', 10),
    verdictRow('npubA', 'TB-07', 'good', 20), // same tester, later → overrides
  ]);
  assert.deepEqual(out['TB-07'], { good: 1, broken: 0 });
});

test('one tester counts once however many reports they send', () => {
  const out = tallyVerdicts([
    verdictRow('npubA', 'TB-07', 'good', 10),
    verdictRow('npubA', 'TB-07', 'good', 11),
    verdictRow('npubA', 'TB-07', 'good', 12),
  ]);
  assert.deepEqual(out['TB-07'], { good: 1, broken: 0 });
});

test('ignores non-verdict rows and malformed/invalid bodies', () => {
  const out = tallyVerdicts([
    { type: 'bug', fromNpub: 'npubA', body: 'doors too tall', ts: 1 },
    { type: 'test-verdict', fromNpub: 'npubB', body: 'not json', ts: 2 },
    { type: 'test-verdict', fromNpub: 'npubC', body: JSON.stringify({ ref: 'TB-07', verdict: 'maybe' }), ts: 3 },
    verdictRow('npubD', 'TB-07', 'good', 4),
  ]);
  assert.deepEqual(out['TB-07'], { good: 1, broken: 0 });
});

test('build filter scopes to one build (per-deploy auto-clear path)', () => {
  const recs = [
    verdictRow('npubA', 'TB-07', 'broken', 10, { build: 'v1' }),
    verdictRow('npubA', 'TB-07', 'good', 20, { build: 'v2' }),
    verdictRow('npubB', 'TB-07', 'good', 21, { build: 'v2' }),
  ];
  // Only v2 verdicts count → the old v1 "broken" is dropped.
  assert.deepEqual(tallyVerdicts(recs, { build: 'v2' })['TB-07'], { good: 2, broken: 0 });
  // No filter → latest-per-npub across builds: npubA good (ts20) + npubB good.
  assert.deepEqual(tallyVerdicts(recs)['TB-07'], { good: 2, broken: 0 });
});

test('output carries only counts — no notes, npubs, or other PII', () => {
  const out = tallyVerdicts([verdictRow('npubA', 'TB-07', 'good', 1, { note: 'secret kid text' })]);
  const json = JSON.stringify(out);
  assert.ok(!json.includes('secret kid text'), 'note text must not leak');
  assert.ok(!json.includes('npubA'), 'npub must not leak');
  assert.deepEqual(out['TB-07'], { good: 1, broken: 0 });
});

test('empty / nullish input yields an empty object', () => {
  assert.deepEqual(tallyVerdicts([]), {});
  assert.deepEqual(tallyVerdicts(null), {});
  assert.deepEqual(tallyVerdicts(undefined), {});
});
