# Multiplayer (LAN)

Axe'n'Stax supports two flavours of multi-player out of the box:

1. **Local split-screen** — 2 to 4 players sharing one machine.
2. **LAN** — multiple machines on the same network connecting to a
   hosted world.

This page covers both. For playing with a friend who **isn't** on your
network — a different house, a different city — see
**[Play with a friend online](play-with-a-friend-online.md)**: new, and not
the same setup as LAN.

## Local split-screen

Up to **4 players** share one keyboard + 3 controllers. The screen
splits into:

- 1 player → full screen
- 2 players → top + bottom halves
- 3 players → 2×2 grid with one quadrant empty
- 4 players → full 2×2 grid

### Joining

When a controller is connected and you press a button on it, an
empty quadrant lights up with a **"Press A to join"** prompt. The
new player picks a colour + drops in. Disconnecting a controller
keeps that player alive (visible) but freezes their input until they
reconnect.

### Web (PWA) split-screen

WASM/Chromium supports split-screen too — same UX, plus a window-
size toast that asks you to enlarge the browser if the windows are
cramped. Performance: the F3 overlay shows tick + frame samples;
expect to see slightly worse perf than native at 4 players.

## LAN

> **Native app only.** Hosting and joining a LAN game are **native desktop app**
> features. On the web build, both Host and Join are replaced with "get the
> desktop app" explainer buttons — there's no functioning LAN host/join UI on
> web at all. (Split-screen itself does work on web — see above.)

### Host

1. Create a world (or open an existing one) in **Survival** or
   **Creative**.
2. On the world's card in the lobby, click **Host**. Once the world has
   loaded, a message tells you the
   address friends should use (your machine's local IP + port 7700). If
   hosting can't start — the port is already in use, or there's no network —
   the message says why, and your world opens for solo play instead.
3. While you host, open the **Pause menu**: the **Hosting on your network**
   panel shows the address again (with a **Copy** button) and how many
   players are in the world. Friends on the same network can pick your
   world from their Join list, or type the address.

### Join

1. From the lobby, click **Join Game**.
2. Pick the host's world from **Games on this network** (it appears when
   your network supports UDP broadcast on port 7705; a game that is full or
   on a different version is greyed out and says why), or type the host's
   `IP:7700`. If the connection fails, you'll see the reason back at the
   lobby.
3. You must be **signed in with Signet** to join — the host requires a
   verified, signed identity and rejects a join attempt that doesn't have
   one. Your player name is just a display fallback; your real identity
   comes from your Signet sign-in. The host sees you under the name they
   have for you in their own contacts. Otherwise they see the name on your
   Signet handle (or the one you typed) followed by a short tag from your
   npub, like `Sam-q3xk`, so nobody can pass as someone the host knows; with
   no name at all, a short form of your npub (like `npub1abcd…wxyz`) —
   never a string of hex. A guest shows as `Sam (guest)`.

### Network requirements

- **UDP port 7700** open on the host (game traffic).
- **UDP port 7705** open on the host (LAN-discovery broadcasts —
  optional; without it you can still type the IP manually).
- Players + host on the same subnet (192.168.x.x typically).

### Limits

| Knob | Default |
|---|---|
| Max remote players | 8 |
| Local split-screen | 4 |
| Tick rate | 20 TPS — server-authoritative |

Mixed mismatched protocols will refuse to connect; if your friend's
client doesn't match the host, both need to upgrade.

## Chat

Type a line **without** a leading `/` in the chat overlay (**T**) and it goes to
the other players in the world — real world chat, not a local echo. It needs
you to be signed in (no verified identity, no chat) and it respects who you've
recognised as family/friends. See **[Chat & Commands](chat-and-commands.md)**
for the full rule. There's still **no in-game voice chat**.

## Feedback

Alpha testers can send a bug or an idea with `/bug` or `/idea` in the desktop app —
it's sent privately to the dev team. It's off by default: open Settings (top of the
world list, or the pause menu) and tap the version line 7 times to switch it on. (The browser version has no feedback channel.)

## What's NOT here yet

- **Internet matchmaking / a public server browser.** There's no directory of
  worlds anywhere to browse — and there won't be. Playing with someone outside
  your network is by direct invite only (**[Play with a friend online](play-with-a-friend-online.md)**),
  never a listing.
- Dedicated server image / Docker container
- Authoritative anti-cheat — the server trusts the local (hosting) player's
  position; remote players are already server-simulated, but full server
  authority over everyone isn't finished yet
- Cross-game allowlists / per-player permission grids
