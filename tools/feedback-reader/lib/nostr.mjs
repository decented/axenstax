// Nostr primitives for the dev reader, plus the SHARED gift-wrap helper.
//
// nip59.cjs reads its primitives from globalThis.AxeNostr; we
// populate that from the npm package and then load it via CJS require, so
// every reader shares ONE wrap/unwrap implementation.
import * as NT from 'nostr-tools';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

globalThis.AxeNostr = {
  nip44: NT.nip44,
  generateSecretKey: NT.generateSecretKey,
  getPublicKey: NT.getPublicKey,
  finalizeEvent: NT.finalizeEvent,
  getEventHash: NT.getEventHash,
  verifyEvent: NT.verifyEvent,
};

const require = createRequire(import.meta.url);
const nip59 = require(
  path.resolve(path.dirname(fileURLToPath(import.meta.url)), './nip59.cjs')
);

export { NT, nip59 };
