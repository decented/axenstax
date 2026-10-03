// Entrypoint for the vendored relay-client IIFE bundle (window.AxeRelay).
//
// Exposes a minimal client shaped exactly for @forgesworn/stash's
// nostrManifestStore RelayClient interface — publish(event) + query(pubkey,
// kind, dTag?) — backed by nostr-tools SimplePool, plus queryByTag(tag, value,
// kind, since) for the lobby mailbox's inbound NIP-17 pull. Also re-exports
// verifyEvent so the manifest can be authenticity-checked, and getEventHash.
import { SimplePool } from 'nostr-tools/pool';
import { verifyEvent, getEventHash } from 'nostr-tools/pure';

/**
 * Make a Stash-compatible RelayClient over one or more relay URLs.
 *   makeRelayClient(['wss://relay.trotters.cc'])
 *     .publish(signedEvent) -> Promise<void>
 *     .query(pubkey, kind, dTag?) -> Promise<NostrEvent[]>  (verified)
 */
export function makeRelayClient(relays) {
  const pool = new SimplePool();
  const urls = Array.isArray(relays) ? relays : [relays];
  return {
    async publish(event) {
      // SimplePool.publish returns one promise per relay; succeed if any does.
      const results = await Promise.allSettled(pool.publish(urls, event));
      if (!results.some((r) => r.status === 'fulfilled')) {
        throw new Error('relay publish failed on all relays');
      }
    },
    async query(pubkey, kind, dTag) {
      const filter = { authors: [pubkey], kinds: [kind] };
      if (dTag !== undefined) filter['#d'] = [dTag];
      // querySync gathers stored events across relays, dedupes by id.
      const events = await pool.querySync(urls, filter, { maxWait: 4000 });
      // Only hand back authentic events — drop anything that fails signature.
      return events.filter((e) => {
        try {
          return verifyEvent(e);
        } catch {
          return false;
        }
      });
    },
    // Query by a single tag filter, e.g. queryByTag('p', recipientHex, 1059).
    // The lobby mailbox uses this to pull inbound NIP-17 replies: gift-wraps are
    // signed by throwaway ephemeral keys, so they can't be found by author — only
    // by the `#p` recipient tag. `since` (unix secs) trims volume; callers dedup
    // by event id because NIP-59 randomises wrap timestamps into the past.
    async queryByTag(tag, value, kind, since) {
      const filter = { kinds: [kind] };
      filter['#' + tag] = [value];
      if (since) filter.since = since;
      const events = await pool.querySync(urls, filter, { maxWait: 4000 });
      return events.filter((e) => {
        try {
          return verifyEvent(e);
        } catch {
          return false;
        }
      });
    },
    close() {
      try {
        pool.close(urls);
      } catch {
        /* best-effort */
      }
    },
  };
}

export { verifyEvent, getEventHash };
