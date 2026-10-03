// Live relay I/O for the dev reader (owner-run; not unit-tested — the live run
// with the real key is the verification boundary). Thin wrapper over nostr-tools
// SimplePool. Inbound feedback is queried/subscribed by the `#p` tag (gift-wraps
// are signed by throwaway ephemeral keys, so we can't filter by author).
import { SimplePool } from 'nostr-tools/pool';
import { verifyEvent } from 'nostr-tools';

export function makeRelay(relayUrls) {
  const urls = Array.isArray(relayUrls) ? relayUrls : [relayUrls];
  const pool = new SimplePool();
  return {
    async publish(event) {
      const results = await Promise.allSettled(pool.publish(urls, event));
      if (!results.some((r) => r.status === 'fulfilled')) {
        throw new Error('relay publish failed on all relays');
      }
    },
    // One-shot catch-up: all gift-wraps addressed to us since `since` (unix secs).
    async fetchGiftWraps(recipientHex, since) {
      const filter = { kinds: [1059], '#p': [recipientHex] };
      if (since) filter.since = since;
      return pool.querySync(urls, filter, { maxWait: 5000 });
    },
    // Resolve a reporter's SIGNED-IN handle from their Signet persona credential
    // (kind-31000, the `display-name` tag) — the SAME source the entrance card
    // and the Hash Dash prefill treat as canonical (see persona-handle.js).
    //
    // We deliberately do NOT read kind-0: the Hash Dash leaderboard publishes a
    // kind-0 with a player-EDITABLE "name to show" on the player's own npub, so
    // a kind-0 lookup logs that typed name, not who-they-signed-in-as. kind-31000
    // is untouched by that box. newest-wins, NIP-40 `expiration` honoured,
    // signature verified. Returns the handle string, or null if none.
    async fetchPersonaHandle(authorHex) {
      const evs = await pool.querySync(urls, { kinds: [31000], authors: [authorHex], limit: 20 }, { maxWait: 4000 });
      if (!evs || !evs.length) return null;
      const now = Math.floor(Date.now() / 1000);
      const tagOf = (e, k) => { const t = (e.tags || []).find((x) => x[0] === k); return t ? t[1] : null; };
      const best = evs
        .filter((e) => e && e.pubkey === authorHex && tagOf(e, 'display-name'))
        .filter((e) => { const x = tagOf(e, 'expiration'); const n = x != null ? parseInt(x, 10) : null; return !(Number.isFinite(n) && n < now); })
        .filter((e) => { try { return verifyEvent(e); } catch { return false; } })
        .sort((a, b) => (b.created_at || 0) - (a.created_at || 0))[0];
      return best ? String(tagOf(best, 'display-name')).slice(0, 100) : null;
    },
    // Persistent subscription: calls onEvent for each new gift-wrap to us.
    subscribe(recipientHex, onEvent, since) {
      const filter = { kinds: [1059], '#p': [recipientHex] };
      if (since) filter.since = since;
      return pool.subscribeMany(urls, [filter], {
        onevent: onEvent,
        oneose() { /* end of stored events — keep listening for live ones */ },
      });
    },
    close() {
      try { pool.close(urls); } catch { /* best-effort */ }
    },
  };
}
