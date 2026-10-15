# Test sheet — animal visual polish (2026-10-15)

Seven species receive distinct models and default coats: cat, fox, crab,
reindeer, parrot, donkey and mule. Automated results and actual headless image
provenance are recorded in `.claude/epic/logs/animal-polish/`; manual steps below
remain pending until a player records them. This sheet makes no claim about
quest/handover acceptance or deployment.

**Automated acceptance:** `./check.sh` passed all 13 gates: 6,528 native tests
passed, 124 were ignored, and the native/web builds, Clippy, licence audit,
texture/pack checks and browser forbidden-symbol gate passed. The compressed
web bundle is 4.61 MiB, below the 5 MiB limit. Three final geometry checks also
passed directly; all 18 final native PNG captures succeeded and passed
independent source/render review. The private comparison gallery was updated
with 25 byte-verified animal images. Optional game-site Playwright smoke,
gallery browser interactions and the live player checks below remain unrun.
This results paragraph follows the frozen tested source; runtime code is
unchanged by the documentation update.

- [ ] A crab looks like a crab: broad low shell, eye stalks, eight legs and
      two open pincers. Watch it walk and turn on sand.
- [ ] Cats are small, with short faces, tabby markings and raised tails.
      Foxes have orange coats and broad cream-tipped tails. Compare both
      beside a wolf. Walk behind the cat and fox: neither has a second face
      on its back.
- [ ] Reindeer have branched antlers and light trim; donkeys have longer ears
      than horses; mules have their own size, ears, mane and tail.
- [ ] Parrots have a hooked beak and coloured wing/tail feathers, with no
      chicken comb. Inspect from the front, side and rear.
- [ ] Walking legs stay attached; baby models scale their new details.
      Check animals at noon, at night and beside a torch; damage flashes work.
- [ ] Feed, tame, breed, leash and interact with applicable species. Ride a
      donkey/mule and use its pack: the visual pass does not change controls.
- [ ] Join another player's world and compare these species with the host's
      view; observe walking, babies and day/night lighting.
- [ ] Open an existing Workshop reskin for one of these species. Existing
      part painting remains attached to the corresponding area; added details
      are selectable. Try a resource pack that replaces a species coat key.

Tested commit / platform:

Species or angle that still looks off:
