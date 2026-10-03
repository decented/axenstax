// Decrypt one inbound kind-1059 gift-wrap (addressed to AxeNStax) into a ledger
// row. Pure + testable: the live relay subscriber (read.mjs) just feeds events
// here. Never throws on a bad event — a malformed/foreign wrap is skipped so the
// long-running subscriber can't be killed by one junk event.
import { NT, nip59 } from './nostr.mjs';

export async function ingest(giftWrap, { signer, ledger, resolveHandle } = {}) {
  let rumor;
  try {
    rumor = await nip59.unwrap(giftWrap, signer);
  } catch (e) {
    console.warn('[reader] skip undecryptable wrap ' + (giftWrap && giftWrap.id) + ': ' + ((e && e.message) || e));
    return null;
  }

  const tag = (k) => {
    const t = (rumor.tags || []).find((x) => x[0] === k);
    return t ? t[1] : undefined;
  };
  // Old contact-form submissions (t=contact; the form was removed 2026-10-03)
  // may still sit in this inbox — skip them so they aren't mis-filed as bugs.
  if (tag('t') === 'contact') return null;
  // Prefer the client report-id (stable across the round trip + dedupes reliably);
  // fall back to the wrap event id for anything that lacks one.
  const reportId = tag('report-id') || (giftWrap && giftWrap.id);
  if (ledger.has(reportId)) return null; // idempotent

  // The reporter's SIGNED-IN handle, so the LOCAL ledger reads "handle · npub ·
  // report" (owner directive: bugs/ideas logged internally — npub + handle +
  // content — never GitHub). Prefer the handle the CLIENT embedded at send time
  // (the Signet session display name the player signed in as): it's always
  // present and immune to a kind-0 the player edited elsewhere (the Hash Dash
  // leaderboard name box). Fall back to resolving their persona credential
  // (kind-31000) from the relay. We NEVER read kind-0. The npub is the
  // cryptographic identity; the handle is a human label logged beside it.
  // Which client sent it. (Replies were removed 2026-10-01 — nobody is ever
  // messaged; status goes out on the public board via status.mjs.)
  const origin = tag('client') || null;
  // Which BUILD sent it (native tags `['build', '<crate version>']` since
  // v0.2.19; web tags it too since 2026-09-03, captured client-side at enqueue
  // from window.__axenstax_feedback_context). A stale install is the first
  // thing to rule out on a bug report — the 2026-07-29 incident was a /bug
  // from a build a month behind the fix.
  const build = tag('build') ? String(tag('build')).slice(0, 32) : null;
  // `origin`, `personaNpub` and `handle` are DISPLAY HINTS — they come straight
  // from tags the sender chose, with no cryptographic binding to their VALUES.
  // The one fact we DO know for certain is which keypair signed the seal
  // (`rumor.pubkey`, captured below as `fromNpub`) — nip59.unwrap already
  // enforces `rumor.pubkey === seal.pubkey`. Since 2026-10-01 native reports
  // are sealed with a one-time burner key and carry no persona/handle at all
  // (status-board spec S5), so `fromNpub` is unlinkable to a person; the
  // persona/handle handling below only serves older native rows and any web
  // rows. They are always shown with an "(unverified)" label in the CLIs
  // (list.mjs, read.mjs, live.mjs) rather than trusted at face value.
  let personaNpub = null;
  const personaHex = tag('persona');
  const personaHexValid = !!(personaHex && /^[0-9a-f]{64}$/i.test(personaHex));
  if (personaHexValid) {
    try { personaNpub = NT.nip19.npubEncode(personaHex.toLowerCase()); } catch { personaNpub = null; }
  }

  let handle = tag('handle') || null;
  // `handleSource` records WHERE the handle came from so a later re-resolve pass
  // (reresolveHandles) can fix old kind-0 rows without ever clobbering an
  // authoritative send-time handle. 'embedded' = the client tagged it; 'persona'
  // = resolved from the kind-31000 credential; null = no handle.
  let handleSource = handle ? 'embedded' : null;
  if (!handle && typeof resolveHandle === 'function') {
    // Native reports seal with a DEVICE key (rumor.pubkey), not the player's
    // persona — resolving the fallback against the seal pubkey would never
    // find a persona credential. When a valid persona tag is present, resolve
    // from THAT hex instead.
    const resolveFrom = personaHexValid ? personaHex : rumor.pubkey;
    try { handle = (await resolveHandle(resolveFrom)) || null; } catch { handle = null; }
    if (handle) handleSource = 'persona';
  }
  if (handle) handle = String(handle).slice(0, 100);

  const rec = {
    id: reportId,
    fromNpub: NT.nip19.npubEncode(rumor.pubkey),
    handle,
    handleSource,
    type: tag('t') || 'bug',
    body: rumor.content || '',
    ts: rumor.created_at || null,
    status: 'new',
    verdict: null,
    issue: null,
    wrapId: giftWrap && giftWrap.id,
    origin,
    personaNpub,
    build,
  };
  ledger.append(rec);
  return rec;
}

// One-shot maintenance pass: re-resolve handles on EXISTING ledger rows from the
// Signet persona credential (kind-31000), fixing rows logged before the
// signed-in-handle fix (which read the player-editable kind-0). A row whose
// handle the client embedded at send time (handleSource 'embedded') is the
// authoritative "who you signed in as" and is NEVER touched. A lookup that finds
// nothing leaves the row as-is (better the old label than no label).
//
//   resolveByNpub(npub) -> Promise<string|null>   the kind-31000 lookup
// Returns { updated, looked, skipped }.
export async function reresolveHandles({ ledger, resolveByNpub, log = () => {} } = {}) {
  let updated = 0, looked = 0, skipped = 0;
  for (const r of ledger.all()) {
    if (r.handleSource === 'embedded') { skipped++; continue; }
    looked++;
    let fresh = null;
    try { fresh = await resolveByNpub(r.fromNpub); } catch { fresh = null; }
    if (fresh) fresh = String(fresh).slice(0, 100);
    if (fresh && fresh !== r.handle) {
      ledger.update(r.id, { handle: fresh, handleSource: 'persona' });
      updated++;
      log(`✏️  ${r.id}: ${r.handle ? '"' + r.handle + '"' : '(none)'} → "${fresh}"`);
    }
  }
  return { updated, looked, skipped };
}
