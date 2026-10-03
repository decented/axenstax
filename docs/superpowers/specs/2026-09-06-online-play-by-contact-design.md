# Online Play by Contact — join a friend's home-hosted world over the internet

**Status:** 🟢 **APPROVED 2026-09-06, BUILDING.** Owner decisions (2026-09-06): design
for the long run — **Signet persona identity is mandatory for online play**; contacts
mirror Signet's list once the persona-scoped view ships upstream; invites fill the
mirror until then; no AxeNStax-operated relay in the game-traffic path, ever.

**Companions:** `../../../../AxeNStax-internal/docs/research/2026-04-01-nat-traversal-multiplayer.md`
(the April research this implements), `docs/foundations/2026-04-20-engine-signet-auth.md`
(join auth, protocol v49), `docs/spec/04-networking.md` §1.8 (multiplayer identity),
`docs/foundations/2026-06-16-heartwood-signed-server-identity.md` (the attestation
pattern reused for players), `src/contacts.rs` + `src/comms.rs` (Kenspeckle tiers),
KithMoot `docs/decisions.md` ("A join link is an invitation", "the ladder").

---

## 0. TL;DR

Hosting a world for a friend in another house works exactly like LAN hosting: the host's
machine is the server, everyone else is a client. What changes is the address. Instead
of typing an IP, a player picks a **contact**, whose address is a **Signet persona npub**.
Nostr relays carry only the encrypted setup handshake (who is calling, and the candidate
addresses each side can be reached at); once both sides have punched through their home
routers the game traffic goes **directly** between the two houses over the existing QUIC
transport, unchanged. No VPS, no port forwarding for most homes, no AxeNStax server in the
path.

Long-run shape, built now:

| Concern | Decision |
|---|---|
| Player address | Signet persona npub. Online play requires a persona on both sides. |
| Per-session signing | A **runtime key** the persona attests once (bunker tap), reusing the server-identity attestation. Signalling is signed by the runtime key; the QUIC join is signed by the persona (existing kind-21236 auth). |
| Contacts | Local mirror (`profile/contacts.json`) with Kenspeckle tiers. Fed by **invites** now, by the Signet persona-scoped contacts view when it ships upstream. |
| Admission | Bearer invite for first contact; thereafter "in my contacts at Kith or closer". Strangers get silence. |
| Signalling | Ephemeral Nostr events, NIP-44 encrypted runtime-key to runtime-key, over a user-editable list of public relays. |
| Reachability | Candidates: LAN, IPv6, UPnP/NAT-PMP mapping, STUN reflexive. Try all in parallel, hand the winning UDP socket to quinn. |
| Fallback | **None in this version.** Plain-words failure message. A forwarder, when it comes, is operator-run software, never ours. |
| Web | Out of scope (web is the anonymous local taster). Native only. |

## 1. Red-line check (load-bearing)

1. **No public directory.** Nothing here lists worlds. A host is reachable only by people
   who hold its invite or are already its contacts. The rendezvous is encrypted; a relay
   sees two runtime pubkeys and timing, nothing about the world.
2. **We operate no relay carrying game traffic.** Relays carry a handful of setup events per
   join. Game traffic is direct. The default relay list is public third-party relays only
   (`server_resolve::PUBLIC_DEFAULT_RELAYS`); `relay.trotters.cc` was removed from it on
   2026-09-28, and a saved list equal to the old default migrates on load. There is no
   TURN/forwarder in this version; when one exists it is self-hosted by an operator.
3. **No central collection of kids' data.** Contacts are a local file. The attestation
   linking a runtime key to a persona travels **inside the encrypted payload**, never as a
   published event, so relays cannot correlate runtime keys to personas. Candidate addresses
   (IPs, which are personal data) are only ever inside NIP-44 ciphertext.
4. **Not a social network.** The lobby column is "Friends & servers": people you already
   know, to build with. No discovery of new people. Copy stays sovereignty/co-building.

Grokster corollary: shipping a self-hostable forwarder later is "facilitate". Nothing here
nudges anyone to disable a safety gate; Signet is mandatory, not disclaimed.

## 2. Vocabulary

- **Persona** — the Signet identity a player signs in with; its npub is the player's address.
- **Runtime key** — a secp256k1 key the game mints per install (reuse the pattern of the former `native_mailbox::key`, removed 2026-10-01 with the reply path:
  minting + file pattern, new file `profile/runtime_key.json`, mode 0600). Signs signalling.
- **Attestation** — the persona's one-time signature over the runtime pubkey. Format:
  `server_identity::attestation` (kind **30420**, version "1") with the tag `role=player`
  added (`role=server` implied when absent; `verify_structural` learns the tag, nothing else
  changes). Minted through the bunker, exactly like `--pair-server`. Stored beside the key.
- **Contact** — `contacts::Contact` extended: `{ pubkey (persona), display_name, tier,
  is_child, runtime_pubkey: Option<[u8;32]>, added_via: Invite|Import|Paste|Kenspeckle,
  added_at }`. `load_local_book()` becomes the union of the Kenspeckle export and the mirror.
- **Invite** — a link + QR minted by a host for one world: version, host persona npub, host
  runtime pubkey, relay list, 16-byte bearer, expiry. Admits once per persona, then that
  persona is a contact.
- **Rendezvous** — the offer/answer exchange over relays.
- **Candidate** — one address the peer may be reachable at, with its type.

## 3. Wire formats

### 3.1 Invite link

```
axenstax://invite/2?h=<host persona npub>&k=<host runtime pubkey hex>
    &r=<relay url>&r=<relay url>&b=<bearer 16 bytes hex>&x=<unix expiry>&w=<world name, urlencoded>
```

QR = the same string (`menu::draw_qr`). Pure `invite.rs`: `Invite { host_persona, host_runtime,
relays, bearer, expires_at, world_name }`, `to_link()`, `parse(&str) -> Result<Invite, InviteError>`
(bad version, missing field, expired, relay not `wss://`, more than 8 relays). Default expiry
**48 hours**; a host may mint a fresh one at any time, which retires the old bearer.

### 3.2 Signalling events

Two ephemeral kinds (NIP-01 ephemeral range, relays do not store them, so both sides must be
online — true for a join by construction):

| Kind | Name | Direction | `p` tag | Content |
|---|---|---|---|---|
| **20900** | `join-offer` | joiner → host | host runtime pubkey | NIP-44 (joiner runtime ↔ host runtime) of an `Offer` |
| **20901** | `join-answer` | host → joiner | joiner runtime pubkey | NIP-44 of an `Answer` |

Outer events are signed by the sender's **runtime** key (`nostr` crate, add feature `nip44`).
Payloads (serde JSON, `v: 1`, append-only fields):

```
Offer  { v, session: hex16, persona: npub, attestation: <kind-30420 event JSON>,
         bearer: Option<hex>, protocol: u32, candidates: [Candidate], sent_at }
Answer { v, session, persona, attestation, accepted: bool, reason: Option<Refusal>,
         protocol, candidates: [Candidate], world_name, sent_at }
Candidate { kind: "lan"|"v6"|"upnp"|"stun", addr: "ip:port" }
Refusal  = NotAContact | BearerInvalid | BearerExpired | ProtocolMismatch | Full   (HostBusy was removed in the final fix wave — never produced)
```

Verification on receipt (both directions, pure `rendezvous::verify_*`): outer signature by
the runtime key; attestation structurally valid and signed by the claimed persona; the
attested runtime pubkey equals the outer signer; `sent_at` within ±120 s; `session` unseen.
Any failure → drop silently (no answer to strangers).

### 3.3 Admission rule (host side, pure `admission.rs`)

```
admit(offer, contacts, active_bearer, now) =
    if contacts.tier_of(offer.persona) <= Kith          -> Accept(AlreadyContact)
    else if offer.bearer == active_bearer && !expired    -> Accept(ByInvite)   // then add contact @ Kith, runtime_pubkey recorded
    else                                                -> Refuse(NotAContact | BearerExpired | BearerInvalid)
```

Kin/Kith admit; **Ken and Stranger do not** (Ken is hear-only in comms; it is not "play in my
world"). Joiner side, on an accepted answer whose persona it does not have: add the host as a
contact at Kith (mutual). A refused answer is shown to the joiner only for `ProtocolMismatch`
and `Full`; `NotAContact` and bearer refusals are **never sent** — the host stays silent.

### 3.4 The QUIC join is unchanged

After the socket is connected the existing handshake runs untouched: `ChallengePacket` →
`JoinRequest` with the persona-signed kind-21236 `auth_event` (bunker) → `access_policy`.
`HostedServer.require_signin` stays `true`. The online host's allowlist is **contacts at
Kith or closer plus this session's bearer-admitted personas**, fed into `access_policy`
exactly as the dedicated server's allowlist is. `PROTOCOL_VERSION` is unchanged (no packet
changes); the `protocol` field in offers exists so a mismatch is explained before a connect.

## 4. Reachability

### 4.1 Candidates (`nat/candidates.rs`, native only)

Gathered on one bound UDP socket (the one quinn will use):

| Kind | How | Dep |
|---|---|---|
| `lan` | every non-loopback IPv4 of the box (outbound-interface trick: `connect` a throwaway UDP socket to a public address, read `local_addr`; plus `if_addrs`-free enumeration is not needed for v1) | none |
| `v6` | the global IPv6 of the box, same trick over v6 | none |
| `upnp` | `igd-next` (blocking API): search gateway (2 s), `add_any_port(UDP, local, lease 7200 s)`, renewed every hour while hosting, removed on stop | **new dep `igd-next`** |
| `stun` | RFC 5389 Binding Request over the same socket to 2 public STUN servers (`stun.l.google.com:19302`, `stun.cloudflare.com:3478`), parse XOR-MAPPED-ADDRESS; hand-rolled (`nat/stun.rs`, ~120 lines, test vectors from RFC 5769) | none |

Order in the list = priority: `lan`, `v6`, `upnp`, `stun`. NAT-PMP is a later addition behind the
same `Candidate` type.

### 4.2 Socket handoff

`network::create_server_endpoint` / `create_client_endpoint` gain variants taking a
pre-bound `std::net::UdpSocket` (`quinn::Endpoint::new(config, server_config, socket,
runtime)`). The socket is bound **before** gathering so STUN's reflexive mapping is the one
QUIC traffic will use. Non-QUIC datagrams (punches, STUN replies) arriving on a quinn socket
are dropped by quinn; STUN is done before the endpoint is created.

### 4.3 The punch (`nat/punch.rs`)

- Host, on an accepted offer: for each joiner candidate, send 3 datagrams (`b"AXNS-PUNCH"` +
  session) 100 ms apart on the server socket. This opens the host NAT toward the joiner.
  Then send the answer.
- Joiner, on the answer: send the same punches to each host candidate, then start quinn
  connects to **all** host candidates in parallel, staggered 150 ms in priority order; the
  first connection to complete the ALPN handshake wins and the rest are aborted. Overall
  deadline **8 s**.
- The server accepts from any address (it already does). Cert verification stays skipped
  (self-signed, as today); the persona-signed join is the trust anchor, plus the existing
  server-identity proof when the host has one.

### 4.4 Failure copy (kid-readable, UK English)

- No answer within 8 s: *"<Name> didn't answer. Are they online with the world open?"*
- Answer, no connect: *"Couldn't reach <Name>'s world. Their router needs UPnP turned on,
  or you both need IPv6. Ask them to check Settings → Online in the game."*
- `ProtocolMismatch`: *"You're on different versions. One of you needs to update."*
- `Full`: *"<Name>'s world is full."*

Host side, at "Host online": if neither `upnp` nor `v6` nor a `stun` candidate could be gathered,
show *"Friends outside your home probably can't reach you. Turn on UPnP on your router."*
and still host (LAN still works).

## 5. Flows

### 5.1 Host

1. World card → **Host online** (native only; greyed with the reason when not signed in
   with a persona, or no runtime attestation yet).
2. First time: mint runtime key → bunker attestation (one phone tap) → store.
3. Bind socket, gather candidates (≤ 3 s, off the main loop), create the quinn endpoint on it,
   `HostedServer::start` with that endpoint (`RemoteTransport::Quic` path, `require_signin = true`).
4. Mint an invite (bearer, 48 h), show the **Online panel**: QR + "Copy invite link" +
   candidate summary ("Reachable: home network, router mapping, internet") + connected list.
5. Relay worker thread: connect to every relay in `online_relays`, `REQ` for kind 20900 with
   `#p = runtime pubkey`, reconnect with backoff; each offer → verify → admit → punch → answer.
   Metrics on the panel: relays connected N/M.
6. Stop hosting: remove UPnP mapping, close REQs, retire bearer.

### 5.2 Joiner

1. Lobby → **Friends & servers** column. Friends list = contacts (Kin/Kith/Ken shown; only
   Kin/Kith have **Join**). **Add a friend**: paste an invite link or an npub. Pasting an
   invite adds a *pending* contact (tier Kith, `added_via: Invite`) and starts a join at once.
2. Join: bind socket, gather candidates, publish the offer to the invite's/contact's relays,
   wait for the answer (8 s), punch, parallel connect, then the existing authed
   `RemoteClient::connect_authed` handshake with the bunker sign driver.
3. Success: `my_servers`-style record on the contact (`last_joined`), toast *"Joined <world>
   at <Name>'s."*

### 5.3 Your address

Lobby panel "Your address": persona npub (`npub…`, never hex — [[feedback_npub_only_display]]),
QR, **Copy**. Explains in one line: *"Give this to a friend so they can invite you."* (Sharing
through mySignet lands when upstream adds copy-npub, ticket #188.)

## 6. Storage and settings

- `profile/runtime_key.json` (0600) — `{ secret_hex }`; `profile/runtime_attestation.json` — the
  signed kind-30420 event.
- `profile/contacts.json` — the mirror (append-only JSON array, `v: 1`).
- `settings.json` gains `online_relays: Vec<String>` (default: `wss://relay.damus.io`,
  `wss://nos.lol`, `wss://relay.primal.net`; public relays only, since 2026-09-28), `online_port: u16` (default 7700; 0 = ephemeral), editable in
  the Graphics/Settings panel under a new **Online** heading.
- Nothing new in `WorldSave`.

## 7. Modules (new, native-gated unless noted)

| Module | Responsibility | Pure/testable |
|---|---|---|
| `invite.rs` | link/QR format, parse, expiry, bearer | pure (cross-platform, trivially) |
| `runtime_identity.rs` | runtime key mint/load, attestation mint (bunker) + verify + store | verify pure |
| `contacts.rs` (extend) | mirror file, union with Kenspeckle export, add/tier/lookup | pure |
| `admission.rs` | the admission rule | pure |
| `rendezvous/{mod,payload,relay_client}.rs` | Offer/Answer types + NIP-44 seal/open + verify chain; relay worker (tokio-tungstenite, multi-relay, REQ/EVENT, backoff) | payload + verify pure; worker behind a trait with an in-memory fake |
| `nat/{candidates,stun,upnp,punch}.rs` | candidate gathering, STUN codec, IGD mapping, punch + parallel connect | stun codec + candidate ordering + connect-race state machine pure |
| `network.rs` (extend) | endpoint constructors from a bound socket | — |
| `online_host.rs` | the host-side orchestration (5.1) | state machine pure over injected traits |
| `online_join.rs` | the joiner-side orchestration (5.2) | same |
| `menu.rs` (extend, keep small: new `friends_ui.rs`) | Friends & servers column, Online panel, Your address | — |

`game_loop.rs` grows only dispatch lines; orchestration lives in the two `online_*` modules.

## 8. Testing

**Unit (TDD, all in `check.sh`):** invite round-trip + every parse error; STUN encode/decode
against RFC 5769 vectors + XOR-MAPPED-ADDRESS v4/v6; Offer/Answer NIP-44 seal/open round trip
and tamper detection; attestation `role=player` verify + wrong-signer rejection; admission
table (contact tiers × bearer states); candidate ordering; connect-race state machine
(first success wins, others aborted, deadline fires, no double-accept); relay-client frame
handling with a fake relay (REQ, EVENT, EOSE, reconnect after close).

**Integration (in-process, no network policy):** two `HostedServer`/`RemoteClient` pairs on
loopback UDP sockets with an in-memory fake relay: full offer → answer → punch → connect →
persona-signed join, plus the refusal paths (stranger silent, expired bearer, protocol
mismatch explained). Uses the existing `TestHost`/channel transport seams where possible.

**Live (owner, test sheet `docs/test-sheets/2026-09-06-online-play.md`):** laptop A on home
wifi hosts; laptop B on a phone hotspot joins by invite link; then B joins again as a
contact without a bearer; then A stops hosting and B sees the "didn't answer" copy. Signer =
any NIP-46 bunker via the paste path (Amber / nsec.app) until mySignet accepts `nostrconnect://`.

## 9. Build phases

1. **P1 Identity + contacts + invite** — `invite.rs`, `runtime_identity.rs`, `contacts.rs`
   extension, `admission.rs`, Your-address panel + Copy. No network yet.
2. **P2 Rendezvous** — payload types, NIP-44 seal/open, verify chain, relay worker + fake,
   settings `online_relays`.
3. **P3 Reachability** — STUN, candidates, `igd-next` mapping, socket handoff into quinn,
   punch + parallel connect race.
4. **P4 Orchestration + UI** — `online_host.rs`, `online_join.rs`, Host-online toggle +
   Online panel, Friends column, failure copy, docs (player guide "Play with a friend online",
   Spec 04 §1.9, operators note), test sheet.
5. **P5 Ship** — `check.sh` green, version bump, AppImage via `tools/packaging/build-local.sh`
   to `~/Desktop`, push main.

## 10. Upstream asks (restated in `forgesworn/signet-plans/MESSAGE-FROM-AXENSTAX.md`)

- Upstream Signet ticket 187 — accept `nostrconnect://` so mySignet can be the native signer.
  **Blocks mySignet for this feature; any NIP-46 bunker works meanwhile.**
  *Update 2026-10-01:* root cause was ours (no `secret` in our QR + ack-only reply
  acceptance), fixed in the vendored `signet-nip46-client`; see
  `docs/foundations/2026-06-10-native-login-login-half-delivered.md`. Pending a live
  phone test, 187 can likely be closed.
- Upstream Signet ticket 188 — copy a persona's npub. Needed to share an address from mySignet.
- Persona-scoped contacts view (2026-09-05 ask) — the import feed for the contacts mirror.

## 10b. Whole-branch review fixes (2026-09-07)

Applied after the final review of the branch. Each is covered by a named
regression test; the wave report is
`.superpowers/sdd/2026-09-06-online-play-by-contact/fix-wave-report.md`.

- **The host's own persona seeds the access allowlist.** `decide_access` reads an
  empty whitelist as "no allowlist gate", so a first-ever online host (no
  contacts, nobody admitted) was open to any signed-in stranger.
  `online_host::access_allowlist` now takes the host persona and can never
  return an empty list.
- **A peer's candidate list is capped at 8 addresses** (`parse_addrs`), on both
  sides. Uncapped, an offer naming thousands of addresses made either machine a
  packet reflector.
- **The relay→consumer channel is bounded** (`sync_channel(256)`, dropping
  `try_send`, counted and surfaced on the Online panel). It was unbounded, so
  anybody who could publish to a subscribed relay could grow the process.
- **`PortMapping` releases on `Drop`**, guarded so an explicit `remove` does not
  release twice, and the window-close path calls `stop_online_on_exit` (which
  waits, bounded — a detached thread does not outlive `main`).
- **A join publishes to the INVITE's relays**, falling back to local settings
  only for a contact call, which carries no invite.
- **A persona switch is named** — both host and join require the attestation to
  name the persona signed in now, and say so, rather than failing as
  "they didn't answer".
- `verify_card` / `verify_server_proof` refuse a `role=player` attestation, as
  `store::store_attestation` already did.
- The contacts mirror is **created** 0600 rather than written then chmod'd.
- The STUN read loop has an overall 1500 ms deadline as well as a per-read one.
- A host runs at most **4 punch workers**; further offers wait for the next poll.
- The module-wide `allow(dead_code)` is gone from every module in this feature.
  `punch::is_punch` moved into its tests (nothing receives a punch — quinn drops
  them), `Refusal::HostBusy` was deleted (never produced), and
  `mint_fresh_invite` is now what "Host online" does when the world is already
  hosted online: a fresh link, the previous bearer retired.
- `OnlineHost::start` / `OnlineJoin::start` hand the relay **back** on failure so
  the caller retires it off the frame.
- Every place that writes the contacts mirror pushes the refreshed book into a
  running `OnlineHost` (`game_loop::push_book_to_online_host`).

## 11. Out of scope (deliberately)

Relay/forwarder fallback (self-hosted forwarder = its own spec); presence ("who is online");
web; voice; kicking/removing a contact beyond deleting them locally; NAT-PMP (follow-on);
Signet contacts import (waits upstream); multiple simultaneous hosted worlds.
