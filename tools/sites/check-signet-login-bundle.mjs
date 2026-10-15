import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { CONSOLE_BUNDLE, readPublishedBundle, selectedVersion } from './signet-login-package.mjs';

try {
  if (process.env.SIGNET_LOGIN_EXPECTED_BUNDLE) {
    throw new Error('Local candidate bundles are unsupported; select a published npm package version.');
  }
  const paths = process.argv.slice(2);
  if (paths.length === 0) paths.push(CONSOLE_BUNDLE);
  const version = selectedVersion();
  const hash = bytes => createHash('sha256').update(bytes).digest('hex');
  readPublishedBundle(version, bundle => {
    const expected = hash(bundle);
    for (const path of paths) {
      if (hash(readFileSync(resolve(path))) !== expected) {
        throw new Error(`${path} differs from published signet-login@${version}; run the strict vendor refresh.`);
      }
    }
    console.log(`Signet Login bundles match the published npm package signet-login@${version}.`);
  });
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
