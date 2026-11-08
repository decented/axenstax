# Test sheet: Trials gallery, postcards, act one reshaped, Trials Island (2026-11-08, v0.2.28)

Covers the `trials-gallery` work merged at 48b83994. The automated gate (`./check.sh`, 7,484 tests) and two GPU harness runs passed on one machine: walk the island into a doorway, play a race, come back to the doorway; and finish a race and keep a postcard. Nobody has played it yet.

Any PIN on a test device or test identity is `123456`.

## Notes for the next release

**Players**
- Trials is now a gallery behind a Trials tile in the lobby, not a list down the left. Each trial is a card with a small block picture; pick one to see what it is, then Play.
- Act one is re-shaped: 34 trials. The ten on the front shelf are open from the start; the rest appear once you first do the thing they build on (fill a bucket, tame something, and so on). Before that, a frosted card hints at what's coming. Newly opened trials wear a "New" badge.
- Eleven trials were folded into others as extra ways to finish them. A trial with several ways lists them under "Pick a way" and ticks the ones you've done. Ticks on the folded-away trials are gone; renamed trials keep theirs.
- Finishing a trial takes a postcard of the moment (no HUD). The newest one shows on the trial's card; the Album in the gallery keeps them all, by shelf. Desktop keeps the newest five per trial; the web keeps the newest one, within a small budget.
- **Trials Island**: "Walk the island" in the gallery. A small island with a doorway for every trial you have opened, its picture built full size beside it. Walk into a doorway to see the card, Play to go, and you come back to the same doorway. The island can't be broken or built on; out past the water you can build.
- Trial progress is saved more safely: written whole or not at all, and an unreadable file is kept aside (`trials.json.bad-<time>`) instead of being overwritten.
- Opening this profile in an older build afterwards may forget which extra trials you'd opened and which ways you'd done them. Completions survive.

## Things to try

Say what you saw, not just pass or fail.

### Gallery
1. Open Trials. Does it feel like a game's menu or a developer's list? What would you change first?
2. A brand-new profile shows the front ten and no hint cards until something is done. Is that right, or should a newcomer see a hint or two straight away?
3. Fill a bucket, tame a wolf, make a dye. Do the right trials appear with "New"? Does the badge go once you've opened the card?
4. Open Camp Cook. Is "Pick a way" clear? Finish it one way: is that way ticked next time?
5. On desktop, does the hero view scroll far enough to see every way? (A small window may cut the last one off.)

### Postcards
6. Finish any trial. Do you get the flash and chime, and is the postcard a good picture of the moment? Colours right (no blue/orange swap)?
7. Album: are the postcards there, newest first? On the web, close and reopen the tab: is the postcard still there?

### Trials Island
8. Walk the island. Is it fun to walk round, or just a menu with legs? Is it too big, too empty, too busy?
9. Walk into a doorway: does the card open, and close when you step back out? Does your mouse or movement get stuck when the card closes? (Known risk on desktop if the cursor ends up over the card; on the web you may need to click back in.)
10. Play from a doorway, finish, choose Back to Trials: are you back at that doorway? Try "Back to menu" too: lobby?
11. Do a trial that opens a new one, then go back to the island. Has a new doorway appeared where its hint plot was?
12. Try to break or place something on the island. Then swim out past the water and build.
13. A gamepad can't press Play on the island card yet. Does that matter to you?

## Questions for Axolittle
- Which ten should be on the front shelf?
- Do any of the merged trials feel worse for losing their own card?
- What should act two's shelf be called, and when should it appear?
