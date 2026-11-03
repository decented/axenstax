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
- An invite link now lets one person into the world you're hosting, until it expires or you stop hosting. If it's forwarded, nobody else gets in. It no longer adds anybody to a Friends list on either side, and `/online copy` makes a new link once the current one has been used.
- Rows that older builds added to a Friends list through an invite are gone on upgrade. Signet friends stay at their Signet tier, but **Join** may ask for an invite link once before it works again. Someone you pasted and then played with on an older build may need pasting again.
- The desktop app's updater never offers a release older than one it has already seen, and it keeps the build it replaces next to the new one as `<name>.AppImage.previous` (about one extra AppImage of disk). That copy is for recovery by hand only: if an update misbehaves, quit the game and rename it back over the AppImage. There is no rollback button.
- Worlds saved by this build won't open on older builds (save format 60). A handover between this build and an older one is refused at the manifest or import, even though the protocol version is the same.
- A joiner who leaves a hosted world comes back where they left, not at spawn, if there is room to stand there. A world remembers its clock across a restart: auctions, bounties and cooldowns carry on from where they were instead of starting over, and the sky resumes its time and weather.

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
4b. With the Signet files damaged, in a window narrower than 1360 logical pixels: is there a Disconnect at the top of the lobby, and does it recover?

### Dedicated server
5. Start with no address set: does the message tell you what to set?
6. Fresh Docker install through the wizard: does it ask for the address and end with a running server? Then skip the wizard and use the dashboard field.
7. Add one npub to the allowlist, remove it, and try to join: are you refused, and does the dashboard make clear why? Then use "allowlist off".
8. `docker stop` with a built thing nearby, start again: is it still there? How long does the stop take on your largest world? (Nobody has timed it. Docker's old 10-second limit would be a problem if it runs long.)

### Browser and sites
9. `/mc-skin` on the web: does a lookup still work? (Mojang's name service was unreliable on 2026-11-01.)
10. Read `/privacy`, `/safety` and `/download` as a parent would. Does anything read wrong?

## Keep slice 2a: the world's clock and where a joiner stands

**Worlds saved by this build won't open on older builds** (save format 60). Keep a copy of a world's folder first if you may go back.

Each needs a real install. Say what you saw.

11. Single-player: play to dusk in rain, save and quit, reopen. Is it the same time of day and the same weather, not morning with a clear sky?
12. Single-player: leave a world, come back a day later. Does a crop, furnace or auction that was part-way carry on from where it was?
13. Host a world, have a friend join, sleep through to a new night and leave. Restart the host and rejoin: are they refused a second sleep the same night?
14. A friend leaves standing on a slab, a low stair step, a snow layer or at a ledge edge. On rejoin, are they back on that spot and not at spawn?
15. A friend leaves in water, in a block, or in a doorway under a low ceiling, then rejoins. Do they come back at spawn, safe?
16. Host Game: a friend leaves about 200 blocks from you and rejoins. Expected: spawn (the host can't check ground it hasn't loaded). Say if that feels wrong.

## Keep slice 2b: lava in newly loaded land

17. Natural lava rims in newly loaded land now flow within minutes instead of up to an hour later, and can start fires sooner. Walk or fly into new land and find a cave wall or surface pool with lava on an open edge. Does the flow look right, and is the fire it starts reasonable?

## Invites: one person, one world

An invite link now lets one person into the world you are hosting, until it expires or you stop hosting. It makes nobody a friend or contact. Automated checks ran on one machine; nobody has tried this on two machines. You need two or three signed-in players on different machines.

18. Host online and send the link to a friend. Do they get in? Afterwards, is either of you in the other's Friends list? (Expected: no.)
19. Send the same link to a second person after the first has joined. What do they see? (Expected: the same "didn't answer" message as anyone with no invite.)
20. The first friend leaves and pastes the same link again. Do they get back in without you doing anything?
21. Open the Online panel after your friend has joined. Is it clear that link is used and how to get another? Type `/online copy`, send the new link to someone else. Do they get in? Does the first friend still get back in on their old link?
22. Stop hosting and host again, or host a different world. Does your friend's old link get them in? (Expected: no. They need a new one.)
23. With a friend in the world by invite, and chat at its default setting, say something each way. Does either of you see it? (Expected: no. A guest isn't a contact.) Does the wording anywhere suggest otherwise?
24. A friend already in your Signet contacts: after upgrading, is their tier unchanged? Does **Join** work, or does it ask for an invite link first? If it asks, does one invite join fix it for good?
25. A friend you added by pasting their address and then played with on an older build: are they still in your list? (Expected: may be gone. Paste them again. Say if that feels wrong.)
26. Read the guide page "Play with a Friend Online" and the Add a friend box. Does anything say an invite makes someone a friend?

## Operator access: small fixes

For anyone running a dedicated server. The Console and the server's admin commands share one identity folder.

27. Operator panel in the game (or the Console's Access box): on a server with no allowlist it should say the allowlist is off and the server is open. Add then remove the only name on the allowlist: it should now say the allowlist is on with nobody listed and that nobody can join. Are the two easy to tell apart?
28. Start a server with `AXENSTAX_WHITELIST` set, then send `whitelist-off`. Expected: it is refused, with a message that the variable keeps the allowlist on, and `whitelist.txt` is untouched. Unset the variable, restart, and `whitelist-off` works. (The Console's *Turn allowlist off* button can't see the variable: it still deletes `whitelist.txt`, and the server stays closed until the variable is unset.)
29. Make `whitelist.txt` mode `0600`, then add and remove a name from the Console and by admin command. Does it stay `0600`?
30. Queue a kick from the Console while the server is running and check the player is removed. Do two kicks queued close together both take effect?

## Saved players and the world save together

Only a hard stop shows this. Automated checks cut a save short at each step. Nobody has pulled the plug on a real machine. Back the world up first.

31. Host a world with a signed-in friend. The friend takes a stack from a chest and stays in the world. Kill the host (`kill -9`, or pull the plug) at any moment, during an autosave if you can catch one (5 minutes on a host, 60 seconds on a dedicated server). Reopen the world. Is the stack in one place only: the chest or the friend's bag, never both or neither?
32. After any hard stop and reopen, look in the world's folder. A `joiners.new` folder should be gone. Is it?
33. Known gap, not a bug: if the friend takes the stack and **leaves**, then the host dies within that same autosave wait, the stack can come back in both places. A clean quit doesn't do this.

## Updater: previous copy and no going back

Needs a real AppImage install and a newer published release.

34. Update in place from the lobby. Is there now a `<name>.AppImage.previous` beside the AppImage, and does the new one start? Update again later: is there still only one `.previous`?
35. Pull the plug (or `kill -9`) during the install if you can catch it. After a restart does the AppImage still start, old or new, never a half file?
36. Known limit: the updater does not go backwards, so a release older than one this install has already seen is never offered. To test it you need to delete `release_mark.json` from the data folder, which resets that memory.

## Known gaps, not bugs in this build

- The privacy page has no contact route for data requests. Owner to supply one.
- The page says the relay's log retention is being confirmed. Needs the fact from whoever runs that relay. The access-log and 3-day retention sentences also rest on the same person's word: nothing in the repo shows them.
- Room-code hosting has no host approval step yet: the code is the gate.
