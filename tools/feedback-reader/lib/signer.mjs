// The AxeNStax signer (holds the secret key). Same shape the kid-side Signet
// signer exposes — { pubkey, nip44:{encrypt,decrypt}, signEvent } — so nip59.cjs
// works against it unchanged.
import { readFileSync } from 'node:fs';
import * as NT from 'nostr-tools';

export function signerFromSecretKey(sk) {
  if (!(sk instanceof Uint8Array) || sk.length !== 32) {
    throw new Error('signerFromSecretKey: a 32-byte Uint8Array secret key is required');
  }
  const pubkey = NT.getPublicKey(sk);
  return {
    pubkey,
    nip44: {
      encrypt: (peerHex, pt) => NT.nip44.encrypt(pt, NT.nip44.getConversationKey(sk, peerHex)),
      decrypt: (peerHex, ct) => NT.nip44.decrypt(ct, NT.nip44.getConversationKey(sk, peerHex)),
    },
    signEvent: (tmpl) => NT.finalizeEvent({ ...tmpl, pubkey }, sk),
  };
}

function hexToBytes(hex) {
  const clean = hex.trim();
  const out = new Uint8Array(clean.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(clean.substr(i * 2, 2), 16);
  return out;
}

// Extract the 32-byte secret from the AxeNStax key file. The file lives OUTSIDE
// the repo (~/.config/axenstax/axenstax-official.json) and is owner-run only.
// Accepts a hex private key under any of the common field names, or an nsec.
export function secretKeyFromKeyFile(filePath) {
  const raw = JSON.parse(readFileSync(filePath, 'utf8'));
  const hexCandidate =
    raw.privkey || raw.privateKey || raw.privateKeyHex || raw.privkeyHex ||
    raw.sec || raw.secretKeyHex || raw.secret_hex || raw.secretHex;
  if (typeof hexCandidate === 'string' && /^[0-9a-f]{64}$/i.test(hexCandidate.trim())) {
    return hexToBytes(hexCandidate);
  }
  const nsecCandidate =
    raw.nsec || raw.nsecKey ||
    (typeof raw.sec === 'string' && raw.sec.startsWith('nsec') ? raw.sec : null);
  if (typeof nsecCandidate === 'string' && nsecCandidate.startsWith('nsec')) {
    const dec = NT.nip19.decode(nsecCandidate.trim());
    if (dec.type === 'nsec') return dec.data; // Uint8Array
  }
  throw new Error(
    'secretKeyFromKeyFile: no recognised private key (looked for a 64-hex privkey/' +
    'privateKey/sec field, or an nsec). Fields present: ' + Object.keys(raw).join(', ')
  );
}

export function loadAxenstaxSigner(filePath) {
  return signerFromSecretKey(secretKeyFromKeyFile(filePath));
}
