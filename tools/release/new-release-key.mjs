#!/usr/bin/env node
// One-time release-key ceremony.
//
// Writes ~/.axenstax-release/release-key.hex (0600 inside a 0700 dir) and
// prints the x-only pubkey + npub. This is a DEDICATED release key, deliberately
// separate from the AxeNStax project identity (the npub that signs Nostr posts,
// key material kept elsewhere on this machine, outside this repo) — same
// separation Vitark keeps between its train-release key and Fathom/Vitark's
// main identity. Reasons to keep them apart:
//   - Blast radius: this key only ever signs kind-30063 release announcements
//     and BUD-02 Blossom upload auth. If it leaks, the fix is "publish a new
//     release announcing a new trust anchor" — not "rotate the identity every
//     other system already trusts."
//   - Different failure mode: the project identity key living outside the
//     repo is about not shipping a secret in git history. This key lives on
//     whichever machine cuts releases, which may not be the same machine (or
//     person) that holds the identity key.
//
// Once run for real, the printed pubkey gets pinned into RELEASE_PUBKEY_HEX in
// release-helpers.mjs and committed — see that file's comment and the README.
//
//   node tools/release/new-release-key.mjs
//
// The secret NEVER leaves ~/.axenstax-release. Do not print it, copy it into
// the repo, or paste it into a chat.
import { generateSecretKey, getPublicKey, nip19 } from 'nostr-tools';
import { mkdirSync, writeFileSync, existsSync, chmodSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

const dir = join(homedir(), '.axenstax-release');
const file = join(dir, 'release-key.hex');
if (existsSync(file)) {
	console.error(`refusing to overwrite ${file} — rotation is a ceremony, delete it yourself first`);
	process.exit(1);
}
const sk = generateSecretKey();
mkdirSync(dir, { recursive: true, mode: 0o700 });
writeFileSync(file, Buffer.from(sk).toString('hex') + '\n', { mode: 0o600 });
chmodSync(file, 0o600);
const pk = getPublicKey(sk);
console.log(`release key written to ${file}`);
console.log(`RELEASE_PUBKEY_HEX = ${pk}`);
console.log(`npub               = ${nip19.npubEncode(pk)}`);
console.log('');
console.log('Next: paste RELEASE_PUBKEY_HEX above into that constant in');
console.log('release-helpers.mjs and commit it — until then publish-release.mjs');
console.log('and query-latest.mjs cannot verify authorship of releases.');
