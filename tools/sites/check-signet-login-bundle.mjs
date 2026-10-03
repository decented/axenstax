import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, join, resolve } from 'node:path';

const root = process.cwd();
const bundlePaths = process.argv.slice(2);

if (bundlePaths.length === 0) {
  console.error('Usage: node tools/sites/check-signet-login-bundle.mjs <bundle-path> [...bundle-path]');
  process.exit(1);
}

function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

const expectedBundle = process.env.SIGNET_LOGIN_EXPECTED_BUNDLE;

const tmp = mkdtempSync(join(tmpdir(), 'signet-login-bundle-'));
const failures = [];

try {
  let expectedPath = expectedBundle ? resolve(root, expectedBundle) : '';
  let expectedLabel = expectedBundle ? 'the Signet Login candidate bundle' : '';
  if (!expectedBundle) {
    const latest = execFileSync('npm', ['view', 'signet-login', 'version'], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();

    const tarballName = execFileSync('npm', ['pack', `signet-login@${latest}`, '--pack-destination', tmp], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim().split('\n').pop();

    const tarballPath = join(tmp, basename(tarballName));
    execFileSync('tar', ['-xzf', tarballPath, '-C', tmp], { stdio: ['ignore', 'ignore', 'pipe'] });

    expectedPath = join(tmp, 'package/dist/signet-login.iife.js');
    expectedLabel = `the latest published release (${latest})`;
  }

  if (!existsSync(expectedPath)) {
    throw new Error(`${expectedLabel} did not contain dist/signet-login.iife.js`);
  }

  const expectedHash = sha256(expectedPath);

  for (const relativePath of bundlePaths) {
    const bundlePath = resolve(root, relativePath);
    if (!existsSync(bundlePath)) {
      failures.push(`${relativePath} is missing`);
      continue;
    }

    const actualHash = sha256(bundlePath);
    if (actualHash !== expectedHash) {
      failures.push(`${relativePath} hash ${actualHash} does not match ${expectedLabel} (${expectedHash})`);
    }
  }

  if (failures.length > 0) {
    console.error(`Vendored signet-login bundles must match ${expectedLabel}.`);
    for (const failure of failures) console.error(`- ${failure}`);
    console.error('Refresh by copying dist/signet-login.iife.js from the expected Signet Login build.');
    process.exit(1);
  }

  console.log(`Vendored signet-login bundles match ${expectedLabel}.`);
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
