// Copy the published npm browser artefact into the operator console only.
// Normal deployment preserves the committed offline copy on registry failure.
// Compatibility checks and deliberate refreshes use --strict to fail instead.
import { writeFileSync } from 'node:fs';
import {
  CONSOLE_BUNDLE, extractLegalNotice, readPublishedBundle, selectedVersion,
} from './signet-login-package.mjs';

try {
  const version = selectedVersion();
  readPublishedBundle(version, (bundle, metadata) => {
    const legal = extractLegalNotice(bundle);
    // Validate the package, checksum and licence before changing either file.
    writeFileSync(CONSOLE_BUNDLE, bundle);
    writeFileSync(`${CONSOLE_BUNDLE}.LEGAL.txt`, legal);
    console.log(`Vendored published signet-login@${version} (tarball sha256 ${metadata.tarballSha256}) into the console.`);
  });
} catch (error) {
  if (process.argv.includes('--strict')) {
    console.error(error.message);
    process.exitCode = 1;
  } else {
    console.warn(`::warning::Could not vendor Signet Login (${error.message}); keeping the committed console bundle.`);
  }
}
