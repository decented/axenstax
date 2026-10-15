import assert from "node:assert/strict";
import { test } from "node:test";
import { getPublicKey, generateSecretKey } from "nostr-tools/pure";
import { decode, npubEncode } from "nostr-tools/nip19";
import { bytesToHex, hexToBytes } from "@noble/hashes/utils.js";

test("published utilities preserve the keeper's persisted hex identity and npub", () => {
  // Public secp256k1 test scalar, never an operational keeper identity.
  const scalar = "0".repeat(63) + "1";
  const bytes = hexToBytes(scalar);
  assert.equal(bytes.length, 32);
  assert.equal(bytesToHex(bytes), scalar);
  const pubkey = getPublicKey(bytes);
  assert.equal(pubkey, "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798");
  const npub = npubEncode(pubkey);
  assert.match(npub, /^npub1/);
  assert.deepEqual(decode(npub), { type: "npub", data: pubkey });
  assert.equal(typeof generateSecretKey, "function");
});

test("published hex decoder rejects malformed persisted key bytes", () => {
  assert.throws(() => hexToBytes("0"));
  assert.throws(() => hexToBytes("gg"));
});
