# Test sheet — Online play by contact (2026-09-06)

**Build:** v0.2.25
**Needs:** two machines, two Signet personas, two different networks (laptop A
on home wifi, laptop B on a phone hotspot). Signer = any NIP-46 bunker via the
paste path (Amber / nsec.app) — mySignet cannot be the native signer until
upstream Signet ticket 187 lands.

## Setup

- [ ] A: sign in with persona A. First "Host online" asks the phone to approve
      this device — approve it.
- [ ] B: sign in with persona B, on a **different network** (phone hotspot).
- [ ] Both: Settings → Online shows a port and four relays.

## 1. Host and invite

- [ ] A: lobby → world card → **Host online**. The world opens.
- [ ] A: a toast says the invite link was copied (or warns that friends outside
      the home probably can't reach you — write down which).
- [ ] A: `/online` prints a reachability line, a relay count, and the link.
      **Reachability said:** ____________________
      **Relays connected:** ____ / ____
- [ ] Send the link to B.

## 2. Join by invite

- [ ] B: lobby → Friends & servers → **Add a friend** → paste the link → Add.
- [ ] B lands in A's world within about ten seconds.
      **How long did it take?** ______
- [ ] Both can see each other move and place blocks.
- [ ] A: the player-inspect view shows B's full npub.

## 3. Join again as a contact

- [ ] B: leave, return to the lobby. A's name is now in the **Friends** list.
- [ ] B: press **Join** next to it — no link, no pasting.
- [ ] B lands in A's world again.

## 4. Nobody home

- [ ] A: leave the world (stop hosting).
- [ ] B: press **Join** again.
- [ ] After about eight seconds B sees:
      *"<Name> didn't answer. Are they online with the world open?"*
      **Exact wording seen:** ____________________

## 5. A stranger gets nothing

- [ ] A: host online again, then press **Host online** again to mint a **fresh**
      invite (this retires the old bearer).
- [ ] B: paste the **old** link.
- [ ] B waits the full eight seconds and gets the "didn't answer" message — NOT
      "your invite expired". (The host must not confirm it is there.)

## 6. Router behaviour

- [ ] A: check the router's admin page — there is a UDP mapping described
      "AxeNStax" while hosting.
- [ ] A: stop hosting. Within a moment the mapping is gone.

## Questions for Axolittle

1. Did the invite link feel like something you'd actually send someone?
2. When it failed, did the message tell you what to do next?
3. Was "Friends & servers" the place you looked for a friend?
4. Anything that felt slow?

## Notes
