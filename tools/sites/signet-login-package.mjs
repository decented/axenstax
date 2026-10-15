// Consume the published browser artefact; never build an upstream checkout.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';

// Update together after reviewing the published release, then refresh strictly.
export const PINNED_VERSION = '0.19.1';
export const PINNED_TARBALL_SHA256 = '3269bfc5fe3b389e5dcef8a8e75824248730d54cde895a862b82f12b100550fe';
export const CONSOLE_BUNDLE = 'tools/sites/console/static/vendor/signet-login.iife.js';

export function packageVersion(value = PINNED_VERSION) {
  // Exact releases and prereleases only: no tags, paths, URLs or npm options.
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[\da-zA-Z-]+(?:\.[\da-zA-Z-]+)*)?$/.test(value)) {
    throw new Error('Supply an exact published signet-login npm version, not a Git ref or tag.');
  }
  return value;
}

export function selectedVersion(env = process.env) {
  const requested = env.SIGNET_LOGIN_PACKAGE_VERSION;
  if (env.SIGNET_LOGIN_CANDIDATE === 'true' && !requested) {
    throw new Error('Publish the candidate to npm first, then supply signet_login_package_version.');
  }
  return packageVersion(requested || PINNED_VERSION);
}

export function readPublishedBundle(version, consume, run = execFileSync) {
  version = packageVersion(version);
  const directory = mkdtempSync(join(tmpdir(), 'axenstax-signet-npm-'));
  const options = { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], maxBuffer: 16 * 1024 * 1024 };
  try {
    const spec = `signet-login@${version}`;
    const packed = JSON.parse(run('npm', [
      'pack', spec, '--ignore-scripts', '--json', '--pack-destination', directory,
    ], options));
    if (packed.length !== 1 || packed[0].name !== 'signet-login' || packed[0].version !== version) {
      throw new Error('npm returned a different package or version.');
    }
    const filename = packed[0].filename;
    if (typeof filename !== 'string' || basename(filename) !== filename) {
      throw new Error('npm returned an invalid tarball filename.');
    }
    const tarball = join(directory, filename);
    const bytes = readFileSync(tarball);
    const hash = createHash('sha256').update(bytes).digest('hex');
    if (version === PINNED_VERSION) {
      if (hash !== PINNED_TARBALL_SHA256) {
        throw new Error(`signet-login@${version} tarball sha256 mismatch; refusing to copy unverified bytes.`);
      }
    } else {
      // A separately selected published candidate must match its registry SRI.
      const integrity = JSON.parse(run('npm', ['view', spec, 'dist.integrity', '--json'], options));
      const match = typeof integrity === 'string' && /^(sha512|sha256)-([A-Za-z0-9+/=]+)$/.exec(integrity);
      if (!match || createHash(match[1]).update(bytes).digest('base64') !== match[2]) {
        throw new Error('Published Signet Login candidate integrity mismatch.');
      }
    }
    // Read just these entries. Never extract an archive into the checkout.
    const manifest = JSON.parse(run('tar', ['-xOf', tarball, 'package/package.json'], options));
    if (manifest.name !== 'signet-login' || manifest.version !== version) {
      throw new Error('Published tarball metadata does not match the requested package.');
    }
    if (manifest.exports?.['./iife'] !== './dist/signet-login.iife.js') {
      throw new Error('Published Signet Login does not export the expected browser artefact.');
    }
    const bundle = run('tar', ['-xOf', tarball, 'package/dist/signet-login.iife.js'], options);
    if (!bundle.trim()) throw new Error('The published Signet Login browser bundle is empty.');
    return consume(bundle, { version, tarballSha256: hash });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

export function extractLegalNotice(content) {
  const marker = '/*! Bundled license information:';
  const start = content.lastIndexOf(marker);
  if (start === -1) throw new Error('The published Signet Login bundle has no bundled licence notice.');
  const end = content.indexOf('\n*/', start);
  if (end === -1) throw new Error('The published Signet Login licence notice is incomplete.');
  const body = content.slice(start + marker.length, end).replace(/\(\*!/g, '/*!').replace(/\*\)/g, '*/');
  return `Bundled license information:${body.replace(/\s+$/, '')}\n`;
}
