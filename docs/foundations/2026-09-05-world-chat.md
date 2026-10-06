# World Chat — text chat in the game, with tiers, a Charter ceiling, guardian copy, and a room plug

**Status:** SPEC · written 2026-09-05 · implements the design decisions settled with the owner
on 2026-09-05 (goal doc, internal repo `docs/goals/2026-09-05-world-chat-overnight.md` §2).
**Scope:** native builds only. The web build carries none of this, by compile-time gate and by
a `check.sh` guard.
**Supersedes nothing.** Extends: Spec 04 (networking), Spec 05 (gameplay/UX), Spec 08 (security).

Out of scope, by decision: proximity voice (its own spec), picture-in-picture video, lobby /
contact DMs, a Rust port of KithMoot, non-Linux URL-scheme registration, any marketing copy.

---

## 0. What this is for

One sentence, and it is the acceptance test:

> A child types in the HUD of a world hosted from the family laptop. A parent reads it and
> replies from the KithMoot app on a phone or a browser tab. An agent owned by the parent's key
> answers in the same conversation, and every line lands back in the child's HUD.

Everything below exists to make that true without AxeNStax becoming the regulated party, and to
leave concrete pieces rather than a demo.

---

## 1. Red lines

The four regulatory red lines in `CLAUDE.md` are load-bearing here, because chat is exactly the
feature that turns a software vendor into a platform. How each is held:

| Line | How this design stays inside it |
|---|---|
| 1 — no AxeNStax-operated directory | No discovery surface is added. A player's contacts are their own list, imported from their own key or a guardian's device. Nothing enumerates worlds, rooms or players. |
| 2 — we operate no game servers, and no relay carrying a group's traffic | Chat rides the operator's own game transport. A room's relays are **operator-supplied**; the code refuses an empty relay list and **lints `relay.trotters.cc` out with a one-line reason**, because trotters is ours and is discovery-hints + sign-in + feedback only. |
| 3 — no central collection of kids' data | The web build has no chat at all. Guardian copy travels child-key → guardian-key over the family's own relays, end-to-end encrypted; nothing of ours is in the middle and nothing is logged anywhere AxeNStax operates. |
| 4 — never a "chat platform" | No site copy changes as part of this work. In-game strings say **"world chat"**. The words "messaging", "social", "chat platform" and "friends list" do not appear on any text surface. |
| Grokster — facilitate, never induce | Chat is gated at a real capability boundary (a verified key, plus a guardian's Charter value), never at a disclaimer. There is no "dev mode", no flag and no config file that lifts a child's ceiling. See §3.5 — the absence of an anonymous-chat path is deliberate and is the main thing to not "helpfully" add later. |

---

## 2. The model

### 2.1 Two directions, not one permission

Hearing and speaking are separate rights. This is the core of the design and the reason the tier
names are worth having.

- **kin** — family. Two-way: hear and speak.
- **kith** — mutually verified acquaintance. Two-way: hear and speak.
- **ken** — one-way recognition. *I have pinned you; you have not pinned me.* **Hear-only.**
  Pinning somebody grants no right to speak to them. This is what makes it safe for a child to
  follow a host or a well-known builder without handing that person a channel to the child.
- **stranger** — no relationship in either direction.

Tiers are **directional and local**: `tier(A → B)` is how A has classified B in A's own address
book. Nothing consults a global graph, because there is no global graph to consult (red line 1).
kin and kith are mutual by construction upstream (both parties verified); ken is one-way by
construction. The engine does not verify mutuality — it trusts each player's own list for that
player's own filtering, which is the only thing it is used for.

### 2.2 Levels

```
CommsLevel = Blocked < Approved < Anyone
```

| Level | Hears | Speaks to |
|---|---|---|
| `Blocked` | nobody — the chat UI is hidden entirely | nobody |
| `Approved` | kin, kith, **and anyone they ken** | kin and kith |
| `Anyone` | everybody in the world | everybody in the world |

### 2.3 The rule (pure function, the heart of the build)

A line from speaker `S` is delivered to listener `L` if and only if **both** hold:

```
speak_ok(S.level, tier(S → L))  AND  hear_ok(L.hear_level, tier(L → S))
```

where

```
speak_ok(Blocked,  _)        = false
speak_ok(Approved, Kin|Kith) = true
speak_ok(Approved, Ken|Str)  = false     // ken grants no right to speak
speak_ok(Anyone,   _)        = true

hear_ok(Blocked,  _)             = false
hear_ok(Approved, Kin|Kith|Ken)  = true  // ken is heard
hear_ok(Approved, Stranger)      = false
hear_ok(Anyone,   _)             = true
```

Two consequences worth stating, because they are the safety properties:

- An `Anyone` speaker does **not** reach an `Approved` listener who has not ken'd them:
  `speak_ok` passes, `hear_ok` fails. A child is protected regardless of who is speaking.
- An `Approved` speaker does **not** reach a stranger even if that stranger is on `Anyone`:
  `speak_ok` fails. A restricted child cannot be drawn into talking to the room.

Evaluation is **server-side, per recipient**. The server never broadcasts a chat line and lets
clients filter — a client that filters is a client that can be patched not to.

### 2.4 A player's own inbound filter

Any player, adult included, may narrow what reaches them without changing what they may say:

```
L.hear_level = min(L.level, L.inbound_filter)
```

`inbound_filter` defaults to the player's own level (i.e. no extra narrowing). Setting it to
`Approved` is the "host only hears the crew" case, and it needs no special host mode: the host
hears kin and kith and whoever they have ken'd, and everyone who has ken'd the host still hears
the host, because the host's own level is untouched. This one field replaces what would
otherwise have been a "broadcast mode".

`inbound_filter` is a client preference sent to the server and held per session. It may only
narrow, never widen — the server clamps it with `min` against the level, so a patched client
gains nothing by sending `Anyone`.

### 2.5 Charter sets the ceiling; the operator can only tighten

```
effective_level(charter, operator) = min(charter, operator)
```

- The guardian's Charter value is the **ceiling** for a child persona, and it travels with the
  child to every world they join.
- A world operator's policy may **tighten** for everyone in that world (`Anyone → Approved →
  Blocked`). It can never loosen. An operator who sets `Anyone` has not granted anything; they
  have merely declined to tighten.
- Recomputed **server-side on every join**, never cached across sessions.
- The full 3×3 table is a unit test.

**Voice will get its own flag.** Do not overload the text flag when proximity voice is specced —
a guardian who permits typing has not thereby permitted an open microphone.

### 2.6 Defaults

| Case | Level | Why |
|---|---|---|
| Persona with a Charter comms value | that value | The guardian said so. |
| Persona with **no** Charter data (cold start, no guardian record) | `Approved` | Never `Anyone`. A missing guardian record is not consent. |
| Natural-person / adult sign-in | `Anyone` | An adult's own key; the operator may still tighten. |
| Operator policy, unset | `Anyone` | The operator tightens deliberately or not at all. |
| **No verified key at all** | **chat unavailable** | See §3.5. |

Note the asymmetry against Charter's usual cold-start posture: Charter mechanism A fails **open**
for static data, because a locked-out child with a dead relay is a worse outcome than a permissive
one for most capabilities. Comms is the exception and fails **closed to `Approved`** — the harm
from a wrongly-open channel is not recoverable, and `Approved` still lets a child talk to their
own family, which is the case that matters when the network is unreliable. This divergence is
deliberate; do not "fix" it into consistency with the other flags.

---

## 3. What the engine holds

### 3.1 Contacts

An in-memory address book, per signed-in player, holding exactly:

```rust
struct Contact {
    pubkey: [u8; 32],       // x-only, hex on the wire, npub for display
    display_name: String,   // sanitised, bounded
    tier: Tier,             // Kin | Kith | Ken
    is_child: bool,         // advisory; drives UI copy only, never a permission
}
```

Nothing else is retained. Any shared secret, private key, token or relay credential present in an
import source is **dropped at the parse boundary and never written to disk** — there is a test
that greps the persisted form for those field names.

`is_child` is advisory: it exists so the UI can say "this is a child's account" where that helps a
guardian, and it must never be an input to `speak_ok` or `hear_ok`. Permission comes from Charter,
which is authoritative; a client-supplied "I am a child" flag is not.

Kenspeckle has no child flag of its own (§8.1). `is_child` is **derived**, and only from the one
place that carries the fact: a `kin` entry whose `relationship` field is `"child"`. Every other
entry yields `false`. This is a labelling convenience and nothing turns on it, which is why
deriving it is acceptable where deriving a permission would not be.

### 3.2 Where contacts come from

In order of preference:

1. **Signet contacts**, if the signed-in key can decrypt them. Signet syncs contacts as a single
   kind-30078 event with `d = signet:contacts`, NIP-44-to-self, authored by the **natural-person**
   key.
2. **A Kenspeckle encrypted export blob**, dropped in the config directory or scanned as a QR from
   the guardian's phone. Format and its surprises are in §8.1 — notably it is *not* NIP-44, and
   Kenspeckle defines no QR transport for it, so this spec defines one.

**Known gap, filed upstream, not worked around:** the game signs in as a *persona*, and a persona
cannot decrypt an event authored by the natural-person key. The correct fix is a persona-scoped,
secret-stripped contacts view from Signet. It is an upstream ask (see §7). It is **not** to be
worked around by touching the natural-person key from the engine — Spec 04 §1.8 forbids exactly
that, and the whole persona boundary exists to make it forbidden.

Until that ships, path 2 is the working path, and it is honest rather than clever: a guardian
exports from their own device and hands the blob to the child's game.

### 3.3 Charter, and the bridge underneath it

One function is the only way the engine learns a comms level:

```rust
fn charter::comms_level(auth: &VerifiedAuth) -> CommsLevel
```

**Checked, 2026-09-05: Charter has no comms capability, and the bridge is therefore the only path
tonight.** This was verified rather than assumed, and the finding is worth writing down because the
word "comms" appears in Charter's README, its integration docs and a source comment, which makes it
look shipped:

- The published SDK type is `ChartedClause { kind: 'schedule', … }` — a single **string literal**,
  not a union. There is no second clause kind to read, and `check()` hardcodes a call to
  `evaluateSchedule`.
- The device-broker wire has fifteen clause kinds (`schedule`, `budget`, `content`, `apps`,
  `learning`, `apprules`, `tethering`, `update`, `lifeline`, `buckets`, `gift`, `maintenance`,
  `standdown`, `listening`, `alwaysavailable`) — **`comms` is not among them.** It is a *reserved*
  row in the prose contract, alongside `spend`, and the NIP-46 method `charter_set_comms` is
  likewise named-but-not-callable.
- Adding it is real work, not a toggle: a closed Rust `ClauseKind` enum plus its hand-mirrored TS
  union, a `GrantComms` body type, bespoke enforcer code (every clause kind has its own), a shared
  test-vector fixture, and a contract-table row.

So the function's body is the **BRIDGE** from day one: a local policy file,
`~/.config/axenstax/guardian-policy.json`, that can only **lower** a player's ceiling, never
raise it (owner decision, 2026-09-28). The file's entry for a subject is clamped to at most
`Approved`: an entry of `blocked` lowers the ceiling; an entry of `anyone` is ignored.

**Why not "signature-verified", as this section first said.** The original bridge verified a
signature against a `guardian_npub` read from *the same file*, so anyone with access to the
filesystem (the child included) could name their own key, sign their own entry and raise their own
ceiling. A signature checked against a key the file names for itself is not a trust root, and
calling it "signature-verified" was a safety claim ahead of the mechanism. Any `guardian_npub` or
`sig` in the file is now ignored. **Raising a ceiling above `Approved` waits for a real capability
boundary**: the Charter comms clause shipping upstream, or a Signet guardian attestation. Marked:

```rust
// BRIDGE: local tightening-only policy file — replace when a real capability
// boundary ships (the Charter comms clause upstream, or a Signet guardian
// attestation); only that may ever RAISE the ceiling above Approved.
// Confined to this function; nothing else reads the file.
```

The bridge is confined to that one function by design, so the cutover is a body swap with no
callers changed.

When the upstream capability ships, the body is swapped for a relay-read and no caller changes.
That is the whole reason for the one-function rule.

**A note on Charter's default posture, because comms deliberately differs.** Charter rev. 7 fails
**open**: no pairing, no clause, or a malformed clause all return `allow: true`, and the source says
so explicitly — *"a broken clause is treated as the absence of a clause"*. The single fail-closed
case is relays configured but never reachable. That is the right call for a screen-time schedule.
It is the wrong call for a comms channel, where a wrongly-open result is not recoverable. §2.6
therefore fails **closed to `Approved`** — not to `Blocked`, because a child who can still talk to
their own family when the network is flaky is the outcome that actually helps. Anyone reconciling
this design with Charter's other flags should leave this difference alone; it is the point.

**Second practical note:** `@forgesworn/charter` is at `0.84.0` in its working tree but its release
workflow runs the npm publish leg as `dry-run` (a private repo cannot do a provenance publish), so
the registry still serves **`0.3.0`**. Anything depending on a published Charter today is depending
on 0.3.0. This is another reason the bridge is the honest path and not a shortcut.

Stored on the player as `PlayerSlot.charter_comms: CommsLevel`, following the shipped
`charter_allows_sats` pattern exactly — same lifecycle, same place in the join path.

### 3.4 Guardian copy

> **Status (audit 2026-10-04): module built, NOT WIRED.** `guardian_copy.rs` (payload shaping, batching) exists and is unit-tested, but nothing in the game loop calls it, the persistent child-HUD indicator is not drawn, and the module carries a blanket `#![allow(dead_code)]` BRIDGE. No chat is copied to a guardian today. Do not describe guardian copy as a shipped feature.

When the Charter value says a child's chat is copied to their guardian, the **child's own client**
sends both directions of the child's conversation to the guardian's key as NIP-17, over the
family's relays, in batches.

- Child → guardian only. There is no room, no keeper, and nothing of ours in the middle.
- Both directions of the child's own conversation: what they said, and what was said to them.
  Not the whole world's chat — a copy is a record of the child's exposure, not surveillance of
  everyone else in the world.
- The child's HUD shows a **persistent, non-dismissable indicator** while copy is on. A child who
  does not know they are being copied is being surveilled; a child who knows is being parented.
  This is a design requirement, not a nicety, and it is also what keeps the feature defensible.
- Batched to keep relay traffic sane; flushed on world exit.

### 3.5 No chat without a verified key

**A player with no verified pubkey has no chat: the UI is hidden and no `ChatMessage` from them is
accepted.** This is an addition to the settled design, and the reasoning matters:

- A tier system without identity is meaningless — an anonymous player is a stranger to everyone
  and would be silent under `Approved` anyway.
- More importantly, admitting anonymous chat would be an **induce-the-bypass path**: a child who
  simply did not sign in would escape their guardian's ceiling. That is precisely the Grokster
  distinction the project holds. The capability boundary must be the key, not a checkbox.

**Defaults (2026-09-28, audit fix).** `ServerPlayer.comms` defaults to `Blocked` — the most
restrictive level. Only a verified player's own Charter/guardian policy, resolved at join
(`resolve_join_comms`), raises it. A guest joiner, a split-screen seat and an unsigned slot 0
therefore hear nothing. Inbound room lines (§4.2, `poll_room`) additionally skip any recipient with
no verified key, mirroring `handle_chat_say`. Previously the default was `Anyone` and `poll_room`
never checked for a key, so room lines from strangers reached exactly the players this section
excludes.

Consequence: a world that admits unauthenticated players (`HostedServer.require_signin == false`,
a dedicated server started with `--allow-guests`; sign-in is its default since 2026-10-06) has chat only for those who did sign in. Operators are told
this plainly, once, in the operator docs — not as a disclaimer, as a fact about what the software
does.

---

## 4. The room plug

The parent is not in the game. They are on a phone. The room is how the game's chat reaches them.

### 4.1 The seam

> **Status (audit 2026-10-04): seam and keeper built, NOT WIRED.** `world_room.rs` (the `WorldRoom` trait, relay lint, NDJSON codec) and `kithmoot_keeper.rs` (`KithMootKeeper`) exist and are tested, but nothing outside those two modules constructs or calls them (`world_room.rs` keeps a blanket `#![allow(dead_code)]`). No world room runs in the live game.

```rust
trait WorldRoom {
    fn start(&mut self, cfg: &RoomConfig) -> Result<RoomHandle>;
    fn post(&mut self, line: &OutboundLine) -> Result<()>;
    fn poll(&mut self) -> Vec<InboundLine>;
    fn invite(&mut self) -> Result<InviteLink>;
    fn rotate(&mut self) -> Result<InviteLink>;
    fn stop(&mut self) -> Result<()>;
}
```

One implementation tonight: `KithMootKeeper`, which supervises a `kithmoot-agent` child process
and speaks its stdio brain seam. **Wrap, don't port** — a Rust KithMoot is a second implementation
later, behind this same trait, with no callers changed.

The trait is the concrete design; the subprocess is the bridge. A test double implementing
`WorldRoom` is what the mirroring tests run against, so the rule logic is testable with no Node,
no relays and no network.

### 4.2 Mirroring, under the same rule

The room is a **member of the world's conversation**, not a bypass of it. A line is mirrored out to
the room only if the room's members would be permitted to hear it under §2.3, evaluated with the
room's own membership standing in as the listener set. A line arriving from the room is delivered
into the world under the same rule, attributed to the room member who sent it.

The consequence to be explicit about: **a child on `Approved` does not have their lines mirrored to
a room whose members they have not ken'd.** A "family room" (members are kin) sees the child's
chat; a "crew room" of the host's acquaintances does not. Both cases are tests.

### 4.3 Agents are owned, by default

**An agent in the room must be owned by a member.** Default, not optional, not a flag.

KithMoot has the mechanism: an ownership proof is minted by a principal's key
(`kithmoot-agent attest`) and rides on every roster entry and message, so the room sees whose agent
it is. But upstream the rule is **off by default** — it is `RoomPolicy.agents ==
"owned-by-members"`, set at room-creation time and carried in the link, and *"a room that says
nothing admits agents as it always did."*

So the default has to be ours to set, and it is: **every room the engine creates sets
`agents: "owned-by-members"`**, and the engine **refuses to attach to a room whose link does not
carry that policy**. Not a warning — a refusal, with the reason printed. An unowned agent in a room
with a child is an anonymous stranger with a language model attached, and the whole point of the
tier system is that there are no anonymous strangers.

This is the thing that keeps an agent in a room with a child from being an anonymous stranger with
a language model attached.

### 4.4 Relays

Operator-supplied. The code:

- **refuses an empty relay list** rather than silently falling back to a default, and
- **lints `relay.trotters.cc` out** of any list it is given, with a one-line reason: *trotters is
  AxeNStax infrastructure for discovery hints, sign-in and feedback; it must never carry a group's
  conversation.*

This is not theoretical tidiness. **KithMoot's own `DEFAULT_RELAYS` is
`['wss://relay.trotters.cc', 'wss://nos.lol', 'wss://relay.primal.net']`** — trotters first. An
engine that passes no relays through gets trotters carrying a family's conversation, which is red
line 2 crossed by omission. Therefore the engine **always passes an explicit relay list** and never
lets the child process fall back to its default. Both halves — the explicit pass and the lint —
have tests.

This is the single piece of this design most likely to be undone by a well-meaning future change
("why not just use the defaults?"). The test names say so.

### 4.5 Commands

- `/room` — status: is a room attached, who is in it, which relays.
- `/room invite` — print and QR the current link.
- `/room rotate` — new epoch; the old link is dead. Wording must say plainly that people admitted
  under the old link stay in unless removed, because that is what actually happens.
- `/room open` — hand off to the browser (§5).

---

## 5. Handoff

The game opens the room link in the browser with the device pass, so a parent moving between the
game machine and their phone does not re-authenticate by hand. The `axenstax://join` URL scheme is
registered by the Linux packager so the reverse direction works too — a link in the browser starts
or focuses the client.

Linux only in this pass. macOS and Windows registration is deferred with the rest of the non-Linux
installer work.

---

## 6. What must not regress

- **The web bundle contains none of the chat surface.** `check.sh` runs
  `tools/smoke/forbidden-symbol.mjs` over the trunk output and **fails the run** on a hit. The web
  build is the anonymous local taster; chat would make it a service.

  Be precise about what that gate can honestly claim. It checks the surface that is *unambiguously*
  native-only — the room plug, guardian copy, the guardian policy file, the contacts QR namespace,
  the wire packet names, the room commands. It deliberately does **not** check for `comms.rs`,
  because that module is compiled on both targets on purpose (§7.8); its presence in the bundle is
  correct, not a leak, and adding it to the forbidden list would be a false tightening.

  The gate also carries a **canary**: a string it knows is in any real bundle. A grep that finds
  nothing is indistinguishable from a grep that cannot see, so if the canary is missing the gate
  **fails** rather than reporting a confident pass — the same principle as `check.sh`'s own rule
  that a gate which skips itself is worse than no gate. Verified 2026-09-05: green on a clean
  bundle, red on a planted marker, canary seen inside the 25 MiB `.wasm`.
- **No client-side permission filtering.** The server decides per recipient.
- **No copy anywhere** — site, store listing, README — describing chat until it has shipped and
  been played. A safety claim ahead of the code is the FTC hook.
- **The trotters lint stays.**
- **`is_child` never reaches the permission functions.**

---

## 7. Where this lands in the existing engine

Verified against the tree at `main@505a798a`. These are facts about the code as it stands, not
aspirations.

### 7.1 The protocol

`PROTOCOL_VERSION` is **59** (`protocol.rs:851`). `PacketType` is a `#[repr(u8)]` enum with manual
discriminants, highest currently `InventoryGrant = 53`. The wire tag is **not** the serde encoding —
`serialize_packet` writes the discriminant as a single leading byte and bincodes the payload after
it, and `deserialize_header` is a **hand-written match**. That match is the real registry: a variant
that is not in it does not exist on the wire.

Two new types, appended (never renumbered — the changelog treats discriminants as a wire-stable
promise):

```rust
ChatSay     = 54,   // client → server: what a player typed
ChatDeliver = 55,   // server → client: one line, already permitted for this recipient
```

Two types rather than one, because they are not the same message. `ChatSay` carries only what the
client is entitled to assert — the text. `ChatDeliver` carries what the server has decided:
attribution, and the kind of line it is. A single symmetric packet would invite a client to assert
its own `from` field.

```rust
pub struct ChatSayPacket {
    pub text: String,
}

pub struct ChatDeliverPacket {
    pub from_pubkey: Option<[u8; 32]>,  // None only for System lines
    pub from_name: String,              // display fallback, server-chosen, never client-asserted
    pub text: String,
    pub kind: ChatWireKind,             // Player | Room | System
}
```

Checklist for the change, in the order the codebase requires:
1. variants on `PacketType` (`protocol.rs:12–53`);
2. arms in `deserialize_header` (`protocol.rs:879–896`) — **nothing dispatches without this**;
3. the payload structs;
4. bump `PROTOCOL_VERSION` 59 → **60** and add a dated changelog entry (`protocol.rs:579+`);
5. a `ChatSay` arm in the `hosted_server.rs` receive match (currently `JoinRequest` / `ClientInput`
   / `Disconnect` / `Ping`, `hosted_server.rs:707–1105`);
6. decode through `safe_deserialize` — there is no per-type size registry, only the blanket
   `MAX_PACKET_SIZE = 65_536`.

### 7.2 Sanitising

There is **no shared string sanitiser** in `protocol.rs`. The `player_name` hardening lives at the
point of use (`hosted_server.rs:718–732`) and is length + control-char **rejection at ingress**.
`sanitize_folder_name` (`save.rs:2878`) and the `take(SIGN_MAX_CHARS)` idiom (`sign.rs:35`) are the
other two existing styles.

Chat follows the `player_name` style — **reject, do not silently truncate** — because a truncated
sentence is a changed sentence, and a player should be told their line did not go rather than have
it arrive altered:

```rust
pub const MAX_CHAT_TEXT_LEN: usize = 256;   // bytes
```

Rejected if: empty or whitespace-only after trimming; longer than `MAX_CHAT_TEXT_LEN`; contains any
`char::is_control()`. A rejected line produces a `System` line back **to the sender only**, saying
which rule it broke. Never a silent drop — a chat that silently eats messages is a chat nobody
trusts.

256 rather than KithMoot's 2 000: this is a line in a HUD over a game, not a document. It also means
a full-rate sender cannot exceed ~7.7 KB/min.

### 7.3 Rate limit

30 lines per minute per player, server-side, token bucket, checked **before** the tier rule (an
over-rate line costs no permission evaluation). Over-rate produces one `System` line to the sender
and then goes quiet until the bucket refills — telling somebody they are rate-limited thirty times
is itself a flood.

30/min matches KithMoot's own `MAX_CHAT_MESSAGES_PER_MINUTE`, so a mirrored room cannot be flooded
by a compliant game and vice versa. Keeping the two numbers equal is deliberate; if one moves the
other should.

### 7.4 The UI

`ChatLineKind` (`commands/dispatch.rs:9–19`) is `Info | Echo | Success | Error | System`. Add:

```rust
Player,   // somebody said something
Room,     // somebody in the attached room said something
```

`ChatState` (`chat_ui.rs:14–28`) already holds a 256-line capped log with `push_log`/`extend_log`,
and `ChatAction::Submit(String)` already comes back from `draw_chat`. Today a non-`/` line is
echoed locally via `ChatLine::echo` and **goes nowhere** (`game_loop.rs:18860–18898`) — chat and
commands run entirely client-local, in-process, with `op_level: OpLevel::Op`. That dead end is
precisely the gap this spec fills: a submitted line that does not start with `/` becomes a
`ChatSay` packet.

**Two gates already sit in front of the overlay and both are inherited as-is:** `GameMode::Playing`
and `state.is_commands_enabled` (`main.rs:1918–1928`). A world created with Commands OFF has no chat,
which is correct and free.

### 7.5 Player state

`PlayerSlot.charter_allows_sats: bool` (`player_slot.rs:123–129`) is the pattern to copy, including
its discipline: it is assigned in exactly one place (`PlayerSlot::new`, `player_slot.rs:287`) and
then **passed as an explicit function parameter** to every consumer rather than read off `self` deep
in a helper. Ten UI modules take it as an argument for exactly that reason.

```rust
pub charter_comms: CommsLevel,
```

Follow the parameter discipline. It is what makes the permission inputs visible in every signature
that depends on them, and it is why the sats flag has never been accidentally bypassed.

Note honestly: `charter_allows_sats` is `true` for everyone today — the join-time wiring its comment
promises was never built. `charter_comms` **must not** repeat that. Its join-time resolution is part
of this work, not a later phase, or the field is decoration.

### 7.6 Server plumbing

`ServerPlayer.verified_pubkey: Option<[u8; 32]>` (`server.rs:103`) is set at join from the verified
auth event (`hosted_server.rs:783`) and is `None` for guests. That `None` is exactly §3.5's "no
chat" case, so the check is one `match` and needs no new state.

There is **no generic broadcast helper** — two patterns exist (a per-tick `broadcasts` queue that
sends to everyone but the source, and direct `self.transports` iteration as in `broadcast_state`).
Chat needs neither: it needs a **per-recipient decision**, so it is a loop that evaluates the rule
for each connected slot and calls `send_to_client` only where it passes. Do not add chat to the
existing broadcast queue; that queue's whole shape is "same bytes to everyone", which is the one
thing chat must not be.

### 7.7 Guardian copy, concretely

`native_mailbox::wire::wrap_report(device: &Keys, target: &PublicKey, rumor: UnsignedEvent)` is
already a general "NIP-17 gift-wrap to a pubkey" primitive despite its name — only `build_rumor` is
mailbox-shaped. Guardian copy builds its own rumor and reuses `wrap_report` + `relay::publish`, and
reuses the module's worker-thread + owned-tokio-runtime idiom (`native_mailbox/mod.rs:160–213`)
rather than inventing a second async send path.

**One decision to make explicit:** the mailbox device key (`key::load_or_mint`, at
`<profile>/mailbox_key.json`; REMOVED 2026-10-01 by the feedback status board) is deliberately **not** the player's persona identity — the persona
secret never lives on the machine. Guardian copy sends from the **device key**, not the persona,
and the guardian's reader is told which device key belongs to which child at pairing time. Sending
"as the persona" would require the persona secret locally, which the whole native-login design
exists to avoid.

### 7.8 The cfg rule — read this before writing a single attribute

This has bitten the project repeatedly. The rule:

- Gate the **module** (`#[cfg(not(target_arch = "wasm32"))] mod native_mailbox;`, `main.rs:132`) or
  gate the **caller**.
- For a pure, cross-platform helper — the tier rule, `effective_level`, the sanitiser, the contacts
  parser — **never** put a bare `#[cfg(not(target_arch = "wasm32"))]` on the function. That deletes
  it from one target, and it is how `-D warnings` breaks in ways that look unrelated. Use:

```rust
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
```

which keeps the item compiled, type-checked and **testable on both targets**, and only silences the
unused lint where it genuinely has no caller yet.

The permission functions are the most important case: they are pure, they are the safety property,
and they must be compiled and tested everywhere even though only native calls them.

### 7.9 The `check.sh` gate

`check.sh` runs version parity → docs tests → `clippy -D warnings` → build → `cargo test --bin` →
`trunk build` → bundle-size gate → optional `--smoke`. Its own header states the principle: *"A gate
that skips itself is worse than no gate."*

Add `tools/smoke/forbidden-symbol.mjs` (Node built-ins only, matching `bundle-size.mjs`) and a
section immediately after the trunk build, greping the built bundle for the chat symbols. Following
the file's convention it must **fail, not skip**, when Node is missing.

---

## 8. External interfaces

### 8.1 Kenspeckle contacts export

Confirmed against the source; two working assumptions were wrong and are corrected here.

**Tiers** are the wire strings `"kin" | "kith" | "ken"`, validated at parse. `kin` and `kith` are
mutual (both carry an ECDH `sharedSecret` from a bond ceremony); **`ken` is one-way by
construction** — `KenEntry` has no shared secret and no reciprocal field, and the source says so:
*"recognition is one-directional."* §2.1's hear-only rule for ken is therefore not a policy we
layered on top; it is the shape of the data.

**The export is not NIP-44.** `exportEntriesEncrypted(entries, key)` is a symmetric self-backup:

```
XChaCha20-Poly1305, raw 32-byte caller-supplied key
layout:  nonce(24) || ciphertext || poly1305_tag(16)
plaintext: JSON array of KindredEntry — no envelope object
```

No ECDH, no conversation key, no KDF inside the library. In Rust this is
`chacha20poly1305::XChaCha20Poly1305` directly, same layout, no custom framing. The 32-byte key is
the consumer's problem — Kenspeckle takes no position on deriving it.

**Fields to keep:** `pubkey`, `displayName`, `tier`. **`is_child` is derived** as `tier == "kin" &&
relationship == "child"`; there is no child flag in Kenspeckle.

**Fields to strip at the parse boundary, by name:**

| Field | Why |
|---|---|
| `sharedSecret` | The ECDH bond secret. Source: *"NEVER published."* The one hard secret. |
| `annotations` | `{groupId, label, note, blocked}` — local-only private notes, explicitly must never reach any wire form. |
| `ownerPubkey` | One of *my own* persona pubkeys, present for anti-correlation. Not a fact about the contact. |
| `bondAssertion`, `provenance`, `corroborations`, `nip05`, `rotation`, `previousPubkeys`, `revoked` | Not secret, but outside the four fields we keep. Keep only what was asked for. |

The stripping test greps the **persisted** form for `sharedSecret`, `annotations`, `note` and
`ownerPubkey` and fails on any hit.

**No test vectors exist** for the backup format — `vectors/` covers the bond ECDH and companion rail
only, and there is no generator for a backup vector. So this spec requires one to be **made**: run
the TypeScript `exportEntriesEncrypted` once over a hand-written fixture roster, freeze the bytes
and the key into `game/engine/assets/test/kenspeckle-export.v1.bin` with the expected parse result
beside it. A cross-implementation format needs a frozen artefact or the two sides drift silently.

**No QR transport exists** for this blob. Kenspeckle uses QR only for small ceremony tokens
(handshake, invite, `signet-grant://pair`), never for the roster. This spec therefore **defines**
one, minimally and in our own namespace so it is not mistaken for a Kenspeckle format:

```
axenstax-contacts:v1:<n>/<total>:<base64url chunk>
```

Fixed 1 200-byte chunks before encoding, reassembled in order, rejected unless every chunk is
present and the total is consistent. It is our format, it is documented here, and if Kenspeckle
later defines its own we adopt theirs and keep this parsing for one release.

### 8.2 The KithMoot stdio seam

`kithmoot-agent` is Node ≥ 22 and **only runs from `dist/`** (`bin/kithmoot-agent.mjs` imports
`../dist/src/node/cli.js`), so `npm ci && npm run build:lib` is a hard prerequisite, not a nicety.

The `stdio` brain (the default for `join`) is **newline-delimited JSON in both directions** — not
length-prefixed. On stdout, `StdioEvent`; on stdin, `StdioCommand`. Everything we need:

```jsonc
// out — first event, always, before anything else
{"type":"ready","participant":"<hex>","device":"<hex>","room":"<id>","url":"<link>","hosting":true}
// out — somebody spoke
{"type":"chat","id":"…","from":"<hex>","name":"Ada","text":"…","sentAt":1757…}
// out — who is present
{"type":"roster","participants":[{"participant":"<hex>","name":"Ada","agent":false,"tracks":[]}]}
// out — we sent something malformed, or an op failed
{"type":"error","message":"…"}
// out — an op succeeded
{"type":"ok","op":"say"}

// in — say something in the room
{"op":"say","text":"…"}
// in — ask who is present
{"op":"roster"}
// in — leave
{"op":"leave"}
```

stderr carries human log lines (`--quiet` silences them) and must be drained regardless, or the
child blocks on a full pipe. The supervisor treats a missing `ready` within a timeout as a failed
start, and restarts with backoff; it never retries a start that failed because the room was
`closed`, because that state is terminal by design upstream.

**A text-only room needs no WebRTC at all** — the media factory is only constructed when `--listen`
is passed. We never pass it. That keeps the dependency surface to relays over TCP.

Room creation (the keeper) and joining are separate processes:

```bash
kithmoot-agent create --base <app>/j/ --name Keeper --brain none \
  --relays <explicit,list> --state <file> --identity <file> \
  --admin <operator-pubkey> --room-name <world>
# link printed, and written to <state>.link, owner-readable only
```

State (`{secret, inviterSk, bearer, epoch, removed, closed, …}`, version 2) persists the room across
restarts; deleting it makes a *new* room, and a `closed: true` room is never reopened. `/room rotate`
is a keeper rekey — only the keeper can do it, epoch increments, and the wording must match what
actually happens: **members already admitted stay in; the old link stops admitting new people.**

Ownership: `kithmoot-agent attest --agent <npub> --identity <key> --label <text>` prints an
`AgentOwnership` JSON (`{agent, principal, issuedAt, expiresAt?, label?, sig}`) to **stdout**. It is
not a Nostr event and is published nowhere; it is a file you pass back as `--owner-proof`. Pin the
agent's key with `--expect-pubkey` — without it, a missing identity file **mints a fresh key
silently**, and an agent running as the wrong key looks exactly like one running correctly.

---

## 9. Build order and what each step must prove

| # | Step | Proof |
|---|---|---|
| 1 | `comms` module: `Tier`, `CommsLevel`, `speak_ok`, `hear_ok`, `deliver`, `effective_level` — pure, no I/O | The **full** tier × level table, both directions, as explicit unit tests. The two safety properties in §2.3 named as their own tests. 3×3 `effective_level` table. |
| 2 | `ChatSay`/`ChatDeliver`, sanitiser, rate limit, server dispatch, per-recipient send | `TestHost`: two players, one child on `Approved`; a stranger's line never reaches the child, a kin's does. Round-trip and rejection tests for the sanitiser. Bucket exhaustion and refill. |
| 3 | HUD: `ChatLineKind::Player`, submit path | Non-`/` submit produces a `ChatSay`; `/` submit still dispatches locally. |
| 4 | `check.sh` forbidden-symbol gate | Gate goes **red** on a deliberately leaked symbol, green after. Prove it fails before trusting it. |
| 5 | `charter_comms` + operator tighten + `min` on join | 3×3 table; bridge file can only lower the ceiling (an `anyone` entry is clamped to `Approved`); upstream ask filed. |
| 6 | `contacts` module | Frozen Kenspeckle fixture parses to exactly the four fields; strip-test greps the persisted form. |
| 7 | Guardian copy | Envelope shape; fires only when gated on; indicator present whenever it is on. |
| 8 | `WorldRoom` + `KithMootKeeper` | Fake-keeper double drives the NDJSON seam. Mirror tests: family room sees the child, crew room does not. Relay lint. Refusal to attach without `owned-by-members`. |
| 9 | Handoff, Haft, release | `/room open` launches; `xdg-open axenstax://join?…` starts the client; `check.sh` green; AppImage published. |

Step 4 before steps 5–8 is deliberate. The gate that keeps this out of the web build should exist
while the surface is small, not be retrofitted once there is a lot to leak.

---

## 10. Open, and honest about it

- **Persona-scoped contacts from Signet** — filed upstream, not worked around. Until it ships, the
  Kenspeckle blob is the path, and a child's contacts arrive from a guardian's device.
- **The Charter comms capability** — filed upstream; the local tightening-only file is the bridge
  and is marked as one.
- **Verified-host badge** (a live key-control challenge, so "the host" cannot be impersonated by a
  chosen display name) — designed for, not built. Display names are already never trusted for
  identity; the npub is the identity, and the inspect view already shows it.
- **Proximity voice** — out, by decision. Nothing here may pre-empt its design, and it gets **its
  own Charter flag**.
