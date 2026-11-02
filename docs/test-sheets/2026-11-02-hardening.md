# Test sheet: access hardening and feedback removal (2026-11-02)

Covers branch `hardening`: in-game feedback removed, dedicated-server and home-host access rules tightened, the final save on shutdown, and public privacy copy. Automated checks ran on one machine. Nothing here has been tried by a person or on two machines yet.

Any PIN on a test device or test identity is `123456`.

## Notes for the next release

**Players**
- `/bug`, `/idea` and `/mailbox` are gone, along with the tester unlock and the Suggestion Box trial. On first launch the game deletes its old report queue and device key from the profile folder.
- A Signet block during Host Game now takes effect straight away and removes the player if they are in the world.
- A contact removed or downgraded in Signet is removed from a hosted world. So are Signet-sourced friends if the contact list goes stale, is disconnected or can't be read.
- If Signet contacts can't be read, Host Game opens the world solo with a notice and Host online shows a notice in the lobby. Pressing Disconnect clears the damaged file. A player whose Signet file was ever damaged in an older build may see this once after upgrading, until they press Disconnect.
- The room code message now says anyone signed in who has the code can join.

**People running a dedicated server: check these before upgrading**
- **Your server needs an address.** A server with a WebSocket listener won't start without `--public-host`, `AXENSTAX_PUBLIC_HOST` or `AXENSTAX_DOMAIN`. A LAN IP or `localhost` is fine. In Docker, the setup wizard or dashboard asks for it. `AXENSTAX_SKIP_WIZARD=1` with no address waits rather than starting.
- **An empty allowlist stays closed.** `whitelist.txt` existing with no valid entries now admits nobody. If you relied on an empty file meaning "no allowlist", delete the file or use `whitelist-off` (also on the console).
- **`require_signin` is strict.** It must say exactly `true` or `false`. An empty or unreadable file stops the server at boot, and on a reload the last good value stays.
- **Removal is live.** Delisting, blocking or requiring sign-in removes players already in the world within about 5 seconds.
- **`docker stop` now saves.** SIGTERM and SIGHUP reach the final save, and a failed final save exits 1. A server started under `nohup` still ignores SIGHUP. Both compose files allow 120 seconds to stop: if you run your own compose file, set `stop_grace_period: 120s`, or a large world's save can be cut off by Docker's 10-second default.
- **The setup wizard tells you what it does to an allowlist.** It opens on "Just my friends" when a list exists, says when your answer turns it off, and its friends box only adds names: remove people on the dashboard.

## Things to try

Each needs a real install. Say what you saw, not just pass or fail.

### Host Game
1. Host Game. Is the "share only with people you know" sentence in the toast and on the pause screen, and does it read well?
2. A friend joins by room code. On your phone, block them in Signet. How long until they're removed, and what do they see? Can they rejoin with the code?
3. Remove a Signet contact while they're in a world you host by contact. Same questions.
4. Turn your phone off for a long session of hosting by contact. Do friends get removed when the contact list goes stale? Is the wording fair? (Owner call: this fails closed and could surprise a host.)

### Dedicated server
5. Start with no address set: does the message tell you what to set?
6. Fresh Docker install through the wizard: does it ask for the address and end with a running server? Then skip the wizard and use the dashboard field.
7. Add one npub to the allowlist, remove it, and try to join: are you refused, and does the dashboard make clear why? Then use "allowlist off".
8. `docker stop` with a built thing nearby, start again: is it still there? How long does the stop take on your largest world? (Nobody has timed it. Docker's old 10-second limit would be a problem if it runs long.)

### Browser and sites
9. `/mc-skin` on the web: does a lookup still work? (Mojang's name service was unreliable on 2026-11-01.)
10. Read `/privacy`, `/safety` and `/download` as a parent would. Does anything read wrong?

## Known gaps, not bugs in this build

- The privacy page has no contact route for data requests. Owner to supply one.
- The page says the relay's log retention is being confirmed. Needs the fact from whoever runs that relay. The access-log and 3-day retention sentences also rest on the same person's word: nothing in the repo shows them.
- Room-code hosting has no host approval step yet: the code is the gate.
