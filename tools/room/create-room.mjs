#!/usr/bin/env node
// Open a world-chat room that requires agents to be owned by a member, and keep
// it open.
//
// WHY THIS EXISTS (and why it should stop existing)
//
//   AxeNStax refuses to attach to a room whose link does not carry
//   `agents: "owned-by-members"`. An agent sitting in a room with a child should
//   be attributable to a person in that room, not an anonymous bot with a
//   language model behind it. Upstream that rule is OFF by default — "a room
//   that says nothing admits agents as it always did" — so making it the default
//   is ours to do.
//
//   KithMoot's library supports it: `CreateRoomOptions.policy`, carried into the
//   link. **Its CLI does not expose a flag for it**, so a room made with
//   `kithmoot-agent create` always has an absent access block, and our engine
//   correctly refuses every such room.
//
//   So this is the smallest possible wrapper: the CLI's own create call, plus
//   the one option it omits. It is not a reimplementation — every moving part
//   (the room, the state, the link, the keeping) is theirs.
//
//   BRIDGE: delete this the day `kithmoot-agent create` takes a policy flag.
//   The ask is filed upstream (signet-plans, MESSAGE-FROM-AXENSTAX §12).
//
// USAGE
//
//   AXENSTAX_KITHMOOT_DIR=<kithmoot checkout> \
//   node tools/room/create-room.mjs \
//     --name "Keeper" --room-name "Home" \
//     --relays wss://your.relay,wss://another.relay \
//     --state $HOME/.config/axenstax/room.json
//
//   Relays are REQUIRED and are yours. There is deliberately no default:
//   KithMoot's own default list begins with relay.trotters.cc, which is
//   AxeNStax infrastructure for discovery hints, sign-in and feedback, and must
//   never carry a family's conversation.
//
//   The link is printed once and written to <state>.link, owner-readable only.
//   Keep the process running: it is what answers the link.
//
// YOU WILL NEED AN OWNERSHIP PROOF FOR THE KEEPER ITSELF
//
//   A keeper sits in the room marked as an agent. In an owned-by-members room
//   an agent with no ownership proof is refused — including this one, which is
//   the room's own keeper. That is the rule working, not a bug: an agent in a
//   room with a child should be attributable to a person, and "it opened the
//   room" is not attribution.
//
//   So the operator attests the keeper's key the same way they attest any other
//   agent, and passes the proof back with --owner-proof:
//
//     node bin/kithmoot-agent.mjs attest \
//       --agent <keeper npub> --identity <YOUR own key file> \
//       --label "world-chat keeper" > keeper-owner.json
//
//   The keeper's npub is printed on its first start (the run that mints its
//   identity file). Nobody but the owner can mint this proof, and this script
//   will never go looking for the owner's key.

import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

function arg(name, fallback = undefined) {
  const i = process.argv.indexOf(`--${name}`);
  return i > -1 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

function die(msg) {
  console.error(msg);
  process.exit(2);
}

const KITHMOOT = process.env.AXENSTAX_KITHMOOT_DIR;
if (!KITHMOOT) {
  die(
    "Set AXENSTAX_KITHMOOT_DIR to your kithmoot checkout.\n" +
      "It must be built first: npm install && npm run build:lib",
  );
}

const base = arg("base", "https://kithmoot.forgesworn.dev/j/");
const name = arg("name", "Keeper");
const roomName = arg("room-name");
const statePath = arg("state");
const relaysRaw = arg("relays", process.env.AXENSTAX_ROOM_RELAYS);

if (!relaysRaw) {
  die(
    "No relays given. Supply your own with --relays wss://a,wss://b (or set\n" +
      "AXENSTAX_ROOM_RELAYS). There is deliberately no default: the library's\n" +
      "default would route your conversation through AxeNStax infrastructure.",
  );
}

const relays = relaysRaw
  .split(",")
  .map((r) => r.trim())
  .filter(Boolean);

// The same lint the engine applies, applied here too, so an operator cannot get
// a room past it by using this script instead of the game.
//
// --allow-trotters is a deliberate, loud, TEST-ONLY escape hatch. It exists so
// that using our own relay is a decision somebody made on purpose and can be
// found later by grepping, rather than a default that quietly drifts. It must
// never be set for a room carrying a family's conversation, and it prints a
// warning every single start so nobody forgets it is on.
const ALLOW_TROTTERS = process.argv.includes("--allow-trotters");
const FORBIDDEN = "relay.trotters.cc";
const kept = [];
for (const url of relays) {
  const host = url.replace(/^wss?:\/\//, "").split("/")[0].split(":")[0].toLowerCase();
  if (host === FORBIDDEN) {
    if (ALLOW_TROTTERS) {
      console.error(
        "WARNING: --allow-trotters is on, so this room is using AxeNStax's own\n" +
          "         relay. That is fine for a test you are running yourself. It is\n" +
          "         NOT fine for a room carrying anyone else's conversation — that\n" +
          "         makes us the operator of the channel. Take this flag off before\n" +
          "         a real family room.",
      );
      kept.push(url);
      continue;
    }
    console.error(
      `dropping ${url} — that relay is AxeNStax infrastructure for discovery ` +
        "hints, sign-in and feedback; it must never carry a group's conversation.",
    );
    continue;
  }
  kept.push(url);
}
if (kept.length === 0) die("No usable relays left after the lint. Supply your own.");

const { RoomAgent } = await import(`${KITHMOOT}/dist/src/agent.js`);
const { localIdentity } = await import(`${KITHMOOT}/dist/src/identity.js`);
const { parseKeeperState, serialiseKeeperState } = await import(
  `${KITHMOOT}/dist/src/keeper-state.js`
);

let state;
if (statePath) {
  try {
    state = parseKeeperState(await readFile(statePath, "utf8"));
  } catch {
    state = undefined; // first run
  }
  if (state?.closed) {
    die(`${statePath}: this room was closed. Delete the state file to make a new one.`);
  }
}

const ownerProofPath = arg("owner-proof", process.env.AXENSTAX_ROOM_OWNER_PROOF);
const identityPath = arg("identity", statePath ? `${statePath}.key` : undefined);

// The keeper's key MUST persist. Without a stable key its npub changes on every
// restart, and an ownership proof the owner minted against the old npub is dead
// — which is the sort of thing you only discover after doing the attest dance
// twice. So: mint once, write 0600, reuse forever, and print the npub every
// start so it is always to hand.
const { generateSecretKey, getPublicKey } = await import(`${KITHMOOT}/node_modules/nostr-tools/lib/esm/pure.js`);
const { npubEncode } = await import(`${KITHMOOT}/node_modules/nostr-tools/lib/esm/nip19.js`);
const { bytesToHex, hexToBytes } = await import(
  `${KITHMOOT}/node_modules/@noble/hashes/utils.js`
);

let skBytes;
if (identityPath) {
  try {
    skBytes = hexToBytes((await readFile(identityPath, "utf8")).trim());
  } catch {
    skBytes = generateSecretKey();
    await writeFile(identityPath, bytesToHex(skBytes) + "\n", { mode: 0o600 });
    console.error(`minted a new keeper identity at ${identityPath}`);
  }
} else {
  die("--state or --identity is required: the keeper's key has to persist.");
}

const keeperNpub = npubEncode(getPublicKey(skBytes));
console.log(`keeper npub: ${keeperNpub}`);

const { normaliseAgentOwnership } = await import(`${KITHMOOT}/dist/src/ownership.js`);
let owner;
if (ownerProofPath) {
  try {
    owner = normaliseAgentOwnership(JSON.parse(await readFile(ownerProofPath, "utf8")));
  } catch (e) {
    die(`could not read the ownership proof at ${ownerProofPath}: ${e.message}`);
  }
} else {
  console.error(
    "No --owner-proof given. An owned-by-members room refuses an agent with no\n" +
      "ownership proof — including its own keeper — so this will fail on join.\n" +
      "Start once to mint the keeper's identity and read its npub, then have the\n" +
      "owner run `kithmoot-agent attest --agent <npub> --identity <their key>` and\n" +
      "pass the result back with --owner-proof.",
  );
}

const agent = await RoomAgent.create({
  base,
  roomName,
  name,
  identity: localIdentity(skBytes),
  owner,
  relays: kept,
  state,
  // The one thing the CLI cannot pass, and the whole reason this file exists.
  //
  // `tier` is REQUIRED, not optional: omitting it does not mean "no tier
  // requirement", it means `policy.tier === undefined`, which is not 'open',
  // so every join is refused with "no kindred proof" — including the keeper's
  // own. `open` is the right pairing here: possession of the link is the
  // admission rule (that is what an invite link IS), while agents still have
  // to show whose they are.
  policy: { tier: "open", agents: "owned-by-members" },
  onState: statePath
    ? (next) => writeFile(statePath, serialiseKeeperState(next), { mode: 0o600 })
    : undefined,
});

if (statePath) {
  if (!state && agent.keeperState) {
    await writeFile(statePath, serialiseKeeperState(agent.keeperState), { mode: 0o600 });
  }
  // The link beside the state, so an operator can `cat` it rather than dig it
  // out of a log. Same mode as the state: a link is a capability.
  await writeFile(`${resolve(statePath)}.link`, agent.url + "\n", { mode: 0o600 });
}

// Print the roster as it changes. The keeper is already in the room and
// admitted, so this is the cheapest way to read full participant pubkeys
// without adding another agent that would itself need an ownership proof.
agent.onRoster((views) => {
  const rows = views.map(
    (v) => `  ${v.participant} ${v.name ? `(${v.name})` : ""}${v.agent ? " [agent]" : ""}`,
  );
  console.log(`roster (${views.length}):\n${rows.join("\n")}`);
});

console.log(`room open, agents owned-by-members. link: ${agent.url}`);
if (statePath) console.log(`link also written to ${statePath}.link`);
console.log("keep this process running — it is what answers the link.");

for (const sig of ["SIGINT", "SIGTERM"]) {
  process.on(sig, async () => {
    try {
      await agent.leave?.();
    } catch {
      /* leaving is best-effort on the way out */
    }
    process.exit(0);
  });
}
