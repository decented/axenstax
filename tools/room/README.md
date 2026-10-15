# Owner-run room wrapper

Use Node 22.13 or newer. Install the exact published identity utilities from the
lockfile, without running dependency lifecycle scripts:

```sh
cd tools/room
npm ci --ignore-scripts
npm test
```

The tests use a public test scalar and package APIs only: no keeper identity
files, rooms, relays or ownership proofs are created or accessed.

`create-room.mjs` resolves `nostr-tools/pure`, `nostr-tools/nip19` and
`@noble/hashes/utils.js` from this directory's npm installation. It no longer
reaches into another checkout's `node_modules` or private package file layout.
The direct versions are pinned; `@noble/hashes` matches the version required by
`nostr-tools`.

KithMoot itself remains an explicit source exception: neither `kithmoot` nor
`@forgesworn/kithmoot` was published on npm when checked on 15 October 2026.
`AXENSTAX_KITHMOOT_DIR` must still identify an independently prepared, built
KithMoot checkout. Its `RoomAgent`, local identity adapter, keeper state and
ownership implementation remain the authority; these npm utilities do not
replace them. See the usage and ownership requirements in `create-room.mjs`.

Running the wrapper opens and keeps a real room. Do that only as an explicit
operator action, with the operator's own relays, persistent keeper identity and
member ownership proof. Installing packages or running the offline tests does
not start it.
