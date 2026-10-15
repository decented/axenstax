import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import {
  CONSOLE_BUNDLE, PINNED_VERSION, extractLegalNotice, packageVersion, readPublishedBundle, selectedVersion,
} from './signet-login-package.mjs';

function registryFixture(overrides = {}) {
  const bytes = Buffer.from('published fixture tarball');
  const state = { directory: null, calls: [] };
  const run = (command, args) => {
    state.calls.push([command, args]);
    if (command === 'npm' && args[0] === 'pack') {
      state.directory = args.at(-1);
      writeFileSync(join(state.directory, 'package.tgz'), bytes);
      return JSON.stringify([{ name: 'signet-login', version: '1.2.3', filename: 'package.tgz', ...overrides.packed }]);
    }
    if (command === 'npm' && args[0] === 'view') {
      return JSON.stringify(overrides.integrity ?? `sha512-${createHash('sha512').update(bytes).digest('base64')}`);
    }
    if (args.at(-1) === 'package/package.json') {
      return JSON.stringify({ name: 'signet-login', version: '1.2.3', exports: { './iife': './dist/signet-login.iife.js' }, ...overrides.manifest });
    }
    if (overrides.missingBundle) throw new Error('published package has no browser bundle');
    return overrides.bundle ?? 'window.Signet = {};';
  };
  return { run, state };
}

test('selects the reviewed pin and requires published exact candidate versions', () => {
  assert.equal(selectedVersion({}), PINNED_VERSION);
  assert.equal(packageVersion('1.2.3-rc.4'), '1.2.3-rc.4');
  assert.throws(() => selectedVersion({ SIGNET_LOGIN_CANDIDATE: 'true' }), /Publish the candidate to npm first/);
  for (const version of ['latest', 'main', 'v1.2.3', '^1.2.3', 'file:../sdk', 'git+https://example.test/sdk', '1.2.3\n', '--ignore-scripts']) {
    assert.throws(() => packageVersion(version), /exact published/);
  }
});

test('reads a verified published artefact without running package scripts, and removes its temporary files', () => {
  const { run, state } = registryFixture();
  const result = readPublishedBundle('1.2.3', (bundle, metadata) => ({ bundle, metadata }), run);
  assert.equal(result.bundle, 'window.Signet = {};');
  assert.equal(result.metadata.version, '1.2.3');
  assert(state.calls[0][1].includes('--ignore-scripts'));
  assert(!existsSync(state.directory));
});

test('package, checksum, metadata and missing-artefact failures never reach the copy callback', () => {
  for (const options of [
    { packed: { name: 'another-package' } },
    { packed: { version: '1.2.4' } },
    { packed: { filename: '../outside.tgz' } },
    { integrity: 'sha512-wrong' },
    { manifest: { version: '9.9.9' } },
    { manifest: { exports: { './iife': './source.js' } } },
    { missingBundle: true },
    { bundle: '  ' },
  ]) {
    const { run, state } = registryFixture(options);
    let copied = false;
    assert.throws(() => readPublishedBundle('1.2.3', () => { copied = true; }, run));
    assert.equal(copied, false);
    assert(!existsSync(state.directory));
  }
  const { run } = registryFixture({ packed: { version: PINNED_VERSION } });
  assert.throws(() => readPublishedBundle(PINNED_VERSION, () => assert.fail('unverified copy'), run), /sha256 mismatch/);
});

test('extracts the upstream licence notices and refuses absent or truncated notices', () => {
  assert.match(extractLegalNotice('code\n/*! Bundled license information:\n(*! MIT *)\n*/'), /\/\*! MIT \*\//);
  assert.throws(() => extractLegalNotice('code'), /no bundled licence/);
  assert.throws(() => extractLegalNotice('/*! Bundled license information:'), /incomplete/);
});

test('strict refresh fails without changing the offline console copy; normal deploy keeps its fallback', () => {
  const root = mkdtempSync(join(tmpdir(), 'axenstax-signet-refresh-test-'));
  const script = fileURLToPath(new URL('./vendor-signet-login.mjs', import.meta.url));
  const target = join(root, CONSOLE_BUNDLE);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, 'old bundle');
  writeFileSync(`${target}.LEGAL.txt`, 'old licence');
  try {
    for (const strict of [true, false]) {
      const result = spawnSync(process.execPath, [script, ...(strict ? ['--strict'] : [])], {
        cwd: root, encoding: 'utf8', env: { ...process.env, SIGNET_LOGIN_PACKAGE_VERSION: 'file:../source' },
      });
      assert.equal(result.status, strict ? 1 : 0);
      assert.match(result.stderr, /exact published/);
      assert.equal(readFileSync(target, 'utf8'), 'old bundle');
      assert.equal(readFileSync(`${target}.LEGAL.txt`, 'utf8'), 'old licence');
      assert(!existsSync(join(root, 'tools/sites/game/static/vendor')));
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('the drift checker refuses an upstream-source bundle override', () => {
  const script = fileURLToPath(new URL('./check-signet-login-bundle.mjs', import.meta.url));
  const result = spawnSync(process.execPath, [script], {
    encoding: 'utf8', env: { ...process.env, SIGNET_LOGIN_EXPECTED_BUNDLE: 'source/dist/sdk.js' },
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Local candidate bundles are unsupported/);
});
