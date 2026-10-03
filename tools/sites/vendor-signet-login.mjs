// Vendor a PINNED signet-login version into the console's vendor dir.
//
// 2026-09-27 audit fix: this used to (a) track `npm view signet-login version`
// (i.e. "latest", unpinned, no integrity check) and (b) also vendor a copy onto
// the GAME site — which no longer loads signet-login at all (the web taster is
// login-free; see game/engine/index.html's note by static/bech32.js). Shipping
// an unreviewed, unpinned npm publish onto a kids' surface that doesn't even use
// it was the exact finding. Fixed both: only the console (which has a real
// operator login) is vendored, and the version + tarball sha256 are pinned
// constants a human bumps deliberately, verified before anything is copied.
//
// Run by deploy.yml BEFORE the WASM build + site sync. It overwrites the committed
// bundle in the runner's checkout (NOT a git commit) so the shipped static asset is
// current; the committed copy in the repo stays as an offline / local-dev fallback,
// refreshed manually per tools/sites/game/static/vendor/REGENERATE.md (the notes
// there still apply even though the game site no longer vendors this package).
//
// NON-FATAL by design: if npm is unreachable, it logs a warning and KEEPS the
// committed bundle so a deploy never breaks just because a registry hiccuped. A
// sha256 MISMATCH is different — that means the pinned version's published
// tarball changed contents (a compromised or republished package), so it is
// FATAL: keep the committed bundle and warn loudly, never install unverified
// content. The hard "are we on latest?" check still lives in
// signet-compatibility.yml.

import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdtempSync, rmSync, existsSync, readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';

// 2026-09-28 audit fix: this vendored bundle had no `.LEGAL.txt` sibling, unlike
// every esbuild-built bundle in `tools/sites/game/static/vendor/`. We don't run
// esbuild here — we copy upstream's own `dist/signet-login.iife.js` byte-for-byte
// (the drift-guard, check-signet-login-bundle.mjs, sha256-compares the committed
// copy against it, so we can't pass `--legal-comments=external` ourselves or
// otherwise alter the file). Upstream's own build still inlines an unminified
// `/*! Bundled license information: … */` comment at EOF, so we extract that
// verbatim into a companion `.LEGAL.txt` instead, matching the sibling bundles'
// format, without touching the byte-for-byte copy itself.
function extractLegalNotice(content) {
  const marker = '/*! Bundled license information:';
  const start = content.lastIndexOf(marker);
  if (start === -1) return null;
  const end = content.indexOf('\n*/', start);
  if (end === -1) return null;
  const body = content.slice(start + marker.length, end);
  // Un-escape the nested comment delimiters esbuild uses to nest per-package
  // license comments inside the outer /*! */ block without closing it early.
  const unescaped = body.replace(/\(\*!/g, '/*!').replace(/\*\)/g, '*/');
  return `Bundled license information:${unescaped.replace(/\s+$/, '')}\n`;
}

// Bump deliberately — re-vendor with `npm view signet-login version` +
// `npm pack signet-login@<version>` + `sha256sum` locally, review the diff,
// then update both constants together. Pinned 2026-09-27 to the then-latest
// published release.
const PINNED_VERSION = '0.17.3';
const PINNED_TARBALL_SHA256 = 'd742309b9832bf0451868c8188486694ddf8fcbf095bc138b7a20cab196aba58';

const TARGETS = [
  'tools/sites/console/static/vendor/signet-login.iife.js',
];

const tmp = mkdtempSync(join(tmpdir(), 'signet-login-vendor-'));
try {
  const tarballLine = execFileSync(
    'npm', ['pack', `signet-login@${PINNED_VERSION}`, '--pack-destination', tmp],
    { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] },
  ).trim().split('\n').pop();
  const tarballPath = join(tmp, basename(tarballLine));

  const actualSha256 = createHash('sha256').update(readFileSync(tarballPath)).digest('hex');
  if (actualSha256 !== PINNED_TARBALL_SHA256) {
    throw new Error(
      `signet-login@${PINNED_VERSION} tarball sha256 mismatch: ` +
        `expected ${PINNED_TARBALL_SHA256}, got ${actualSha256} — ` +
        `refusing to install unverified content (re-pin deliberately if this is an expected republish)`,
    );
  }

  execFileSync('tar', ['-xzf', tarballPath, '-C', tmp], { stdio: ['ignore', 'ignore', 'pipe'] });

  const dist = join(tmp, 'package/dist/signet-login.iife.js');
  if (!existsSync(dist)) {
    throw new Error(`signet-login@${PINNED_VERSION} has no dist/signet-login.iife.js`);
  }
  const distContent = readFileSync(dist, 'utf8');
  const legalNotice = extractLegalNotice(distContent);
  for (const t of TARGETS) {
    copyFileSync(dist, t);
    if (legalNotice) writeFileSync(`${t}.LEGAL.txt`, legalNotice);
  }
  console.log(`Vendored signet-login@${PINNED_VERSION} (sha256 verified) into ${TARGETS.length} site vendor dir(s).`);
} catch (err) {
  // Don't fail the deploy — ship the committed fallback bundle instead. This
  // also covers a genuine sha256 mismatch (thrown above): never install
  // unverified content, but a deploy shouldn't hard-fail over it either.
  console.warn(
    `::warning::Could not vendor signet-login@${PINNED_VERSION} (${err.message}); ` +
      `shipping the committed bundle. Re-vendor manually when convenient (REGENERATE.md).`,
  );
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
