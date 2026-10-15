import assert from 'node:assert/strict';
import { webcrypto } from 'node:crypto';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const source = readFileSync(new URL('./static/console-login.js', import.meta.url), 'utf8');

test('the published browser artefact loads the console API without connecting a signer or relay', () => {
  const context = { crypto: webcrypto, TextEncoder, TextDecoder, URL, Uint8Array, Uint32Array, ArrayBuffer };
  context.window = context;
  const bundle = readFileSync(new URL('./static/vendor/signet-login.iife.js', import.meta.url), 'utf8');
  vm.runInNewContext(bundle, context, { timeout: 5000 });
  assert.equal(typeof context.Signet.login, 'function');
  assert.equal(typeof context.Signet.handleRedirectCallback, 'function');
});

async function browser({ session, callback = { kind: 'no-callback' } } = {}) {
  let click;
  const status = { textContent: '' };
  const button = { disabled: false, addEventListener: (name, handler) => { click = handler; } };
  const requests = [];
  const location = { href: '' };
  const context = {
    document: {
      querySelector: () => null,
      getElementById: id => id === 'status' ? status : button,
    },
    window: {
      location,
      Signet: {
        login: async options => { requests.push({ login: options }); return session; },
        handleRedirectCallback: async () => callback,
      },
    },
    fetch: async (path, options) => {
      requests.push({ path, options });
      return { ok: true, json: async () => ({ challenge: 'server-issued-challenge' }) };
    },
  };
  vm.runInNewContext(source, context);
  await new Promise(resolve => setImmediate(resolve));
  return { click, status, button, requests, location };
}

test('a published SDK login forwards the server challenge and only its auth proof to console verification', async () => {
  const proof = { kind: 21236, sig: 'fixture' };
  const b = await browser({ session: { authEvent: proof, displayName: 'Operator' } });
  await b.click();
  assert.equal(b.requests[0].path, 'auth/challenge');
  assert.equal(b.requests[1].login.challenge, 'server-issued-challenge');
  assert.equal(b.requests[1].login.relayUrl, 'wss://relay.damus.io');
  assert.equal(b.requests[2].path, 'auth/verify');
  assert.deepEqual(JSON.parse(b.requests[2].options.body), { authEvent: proof, displayName: 'Operator' });
  assert.equal(b.location.href, 'dashboard');
});

test('cancelled SDK login does not submit a proof and enables another deliberate sign-in', async () => {
  const b = await browser({ session: null });
  await b.click();
  assert.equal(b.requests.some(request => request.path === 'auth/verify'), false);
  assert.equal(b.button.disabled, false);
  assert.equal(b.status.textContent, 'Sign-in cancelled.');
  assert.equal(b.location.href, '');
});

test('published tagged redirect sessions reach server verification', async () => {
  const proof = { kind: 21236, sig: 'redirect-fixture' };
  const b = await browser({ callback: { kind: 'session', session: { authEvent: proof } } });
  assert.equal(b.requests[0].path, 'auth/verify');
  assert.deepEqual(JSON.parse(b.requests[0].options.body), { authEvent: proof });
  assert.equal(b.location.href, 'dashboard');
});

test('non-session redirect results do not submit an authentication request', async () => {
  for (const callback of [{ kind: 'no-callback' }, { kind: 'denied' }, { kind: 'invalid', reason: 'fixture failure' }]) {
    const b = await browser({ callback });
    assert.deepEqual(b.requests, []);
    assert.equal(b.location.href, '');
  }
});
