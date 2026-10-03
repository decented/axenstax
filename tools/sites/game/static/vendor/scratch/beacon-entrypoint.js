// Entrypoint for the vendored Beacon IIFE bundle (window.AxeBeacon).
// Exposes the core factory + the Nostr verify helpers so AxeNStax can
// build, publish, and verify public content-distribution manifests.
export { createBeacon } from '@forgesworn/beacon';
export { verifyEvent, BEACON_MANIFEST_KIND } from '@forgesworn/beacon/nostr';
