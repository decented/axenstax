// Entrypoint for the vendored Stash IIFE bundle (window.AxeStash).
// Exposes the core factory + bridge manifest store, and the Nostr
// destination-tier manifest store, so AxeNStax can use either tier.
export { createStash, httpManifestStore, blobHashOf, bytesToBase64, base64ToBytes } from '@forgesworn/stash';
export { nostrManifestStore, STASH_MANIFEST_KIND } from '@forgesworn/stash/nostr';
