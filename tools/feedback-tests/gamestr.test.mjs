// Tests for static/gamestr.js — the opt-in gamestr leaderboard publish.
//
// gamestr.js is browser-first (DOM overlay + live signer/relay), but its event
// builders and the consent-gated publishScore() are pure/injectable so they run
// headless here with a recording fake signer + relay. No external deps needed.
//
// Run: node --test gamestr.test.mjs   (or `npm test` to run the whole suite).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const require = createRequire(import.meta.url);
const here = path.dirname(fileURLToPath(import.meta.url));
const { buildScoreEvent, buildHandleEvent, publishScore } = require(
  path.resolve(here, '../sites/game/static/gamestr.js')
);

// Records every signEvent template; returns a plausibly "signed" event.
function recordingSigner() {
  const calls = [];
  return {
    calls,
    signEvent: async (tmpl) => {
      calls.push(tmpl);
      return { ...tmpl, id: 'fakeid', sig: 'fakesig', pubkey: 'fakepk' };
    },
  };
}
function recordingRelay() {
  const published = [];
  return { published, publish: async (ev) => { published.push(ev); } };
}

test('buildScoreEvent is NIP-133 kind 33334 with d-tag + plain-number content', () => {
  const ev = buildScoreEvent('axenstax-hash-dash', 4200);
  assert.equal(ev.kind, 33334);
  assert.deepEqual(ev.tags, [['d', 'axenstax-hash-dash']]);
  // content is a PLAIN stringified number (NIP-133 default; the form every live
  // gamestr board uses — 2048/asteroids/melrise). NOT category-JSON {"work":N},
  // which the board couldn't render.
  assert.equal(ev.content, '4200');
  assert.equal(typeof JSON.parse(ev.content), 'number');
});

test('buildScoreEvent floors + clamps work to a non-negative integer', () => {
  assert.equal(buildScoreEvent('g', 12.9).content, '12');
  assert.equal(buildScoreEvent('g', -5).content, '0');
  assert.equal(buildScoreEvent('g', NaN).content, '0');
});

test('buildHandleEvent is a minimal kind-0 carrying only the name', () => {
  const ev = buildHandleEvent('AxoBuilder');
  assert.equal(ev.kind, 0);
  assert.deepEqual(ev.tags, []);
  assert.equal(ev.content, JSON.stringify({ name: 'AxoBuilder' }));
});

test('consent OFF publishes NOTHING (privacy invariant)', async () => {
  const signer = recordingSigner();
  const relay = recordingRelay();
  const res = await publishScore({ signer, relay }, { gameId: 'g', work: 100, handle: 'X', consent: false });
  assert.equal(res.published, false);
  assert.equal(signer.calls.length, 0, 'signEvent must never be called without consent');
  assert.equal(relay.published.length, 0, 'nothing published without consent');
});

test('consent ON with a handle publishes kind-0 then kind-33334', async () => {
  const signer = recordingSigner();
  const relay = recordingRelay();
  const res = await publishScore(
    { signer, relay },
    { gameId: 'axenstax-hash-dash', work: 77, handle: 'Axo', consent: true }
  );
  assert.deepEqual(res.kinds, [0, 33334]);
  assert.equal(signer.calls.length, 2);
  assert.equal(signer.calls[0].kind, 0);
  assert.equal(signer.calls[1].kind, 33334);
  assert.equal(relay.published.length, 2);
});

test('consent ON without a handle publishes only kind-33334', async () => {
  const signer = recordingSigner();
  const relay = recordingRelay();
  const res = await publishScore({ signer, relay }, { gameId: 'g', work: 5, handle: '', consent: true });
  assert.deepEqual(res.kinds, [33334]);
  assert.equal(signer.calls.length, 1);
  assert.equal(signer.calls[0].kind, 33334);
});

test('consent ON with a whitespace-only handle skips the kind-0', async () => {
  const signer = recordingSigner();
  const relay = recordingRelay();
  const res = await publishScore({ signer, relay }, { gameId: 'g', work: 5, handle: '   ', consent: true });
  assert.deepEqual(res.kinds, [33334]);
  assert.equal(signer.calls.length, 1);
});

test('missing signer with consent throws (caught by the caller)', async () => {
  await assert.rejects(
    () => publishScore({ relay: recordingRelay() }, { gameId: 'g', work: 1, consent: true }),
    /no signer/
  );
});
