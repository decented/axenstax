# Test sheet: new terrain, graphics settings, building variants and pick block (2026-10-24)

Four pieces of work shipped together after one overnight session. Each one
passed `./check.sh` on its own branch, and the combined build passed it again
(see "Automated acceptance"). Nothing below has been played by a person yet.

**Automated acceptance:** `./check.sh` passed all 13 gates on the
combined build (commit 91043a54): 6,823 native tests passed (172 ignored);
clippy, native and web builds, licence audit, forbidden-symbol gate and the
5 MiB web bundle limit all passed. The Playwright smoke run was not run.
Terrain, graphics and building variants each also passed the full gate on
their own branches. Nothing in this sheet has been played by a person yet.

## 1. New worlds: terrain generator v3

Only a **new** world uses the new generator. Old worlds keep the old terrain
forever (byte-identical), so make a fresh world for these checks. The world is
now 256 blocks tall (it was 96).

- [ ] Walk and fly around a new world. Can you find continents, open sea,
      deep ocean (dark, much deeper water), rivers, beaches, plains, savanna,
      desert, taiga, snowy tundra, jungle and real mountains (peaks up to about
      y 200, snow on top)?
- [ ] Cliffs: find a steep escarpment or a cliff with an overhang. Does it
      look natural or like a wall? Is the underside of an overhang lit
      properly (not black)?
- [ ] Rivers: follow one. **Known:** rivers fade out just before the coast
      and above about y 95–110, rather than running into the sea. Does that
      look wrong enough to fix?
- [ ] Biome size: climate regions are large (about 1,000+ blocks across).
      Too big, or good for exploring?
- [ ] Crabs on beaches; no cows, horses, donkeys or mules there.
- [ ] Caves are still the old flooded ones and trees are the old tree rules
      (both are frozen for these worlds; dry caves and new tree spacing will
      come as a later generator). Anything odd where old trees meet the new
      ground, e.g. trees floating or buried on cliffs?
- [ ] Join a v3 world from a second machine: does terrain at the seams match,
      and do tall mountains stream in?

## 2. Graphics settings and performance

- [ ] First run on a fresh install (or with the settings file deleted): which
      preset did it pick for this machine? Is the picture acceptable?
- [ ] An existing settings file keeps the same look as before (nothing
      should change for a player who already had settings).
- [ ] Auto quality (governor): on a slow machine, does it drop render distance
      / scale to hold the frame rate, and step back up when things are quiet?
      On a 75 Hz or 90 Hz monitor with a frame cap of 60, the settings note
      should NOT flicker and terrain should NOT pop in and out every minute.
- [ ] Leaves: Fast (default on Low and Potato): do canopies look acceptable
      (holes, colour)?
- [ ] The draw range is now round: on Low, look along a diagonal. Does the
      fog hide the edge?
- [ ] Load another world from the lobby: no leftover leaves or glass from the
      previous world floating in the new one.
- [ ] Joined to someone else's world, your render distance is never cut by
      auto quality.

## 3. Building variants

New wood and stone stairs, slabs, vertical slabs, doors, trapdoors, fence
gates and walls (block ids 320 and up). No new textures.

- [ ] Find them in the creative tab and with `/give`. Craft a few in survival.
- [ ] Place each shape facing all four ways; doors and trapdoors open and
      close; gates open; walls connect.
- [ ] Walk up stairs, stand on half slabs, walk past vertical slabs and walls.
- [ ] Break them: each drops itself once (no double drops from door halves).
- [ ] **Changed:** stone slabs, stairs and walls now need a pickaxe to drop.
- [ ] **Changed:** wooden doors and trapdoors no longer burn (fence gates do);
      all of them still work as furnace fuel.
- [ ] **Known:** an old save opened in this build works; a save from this
      build will NOT open in an older build ("needs a newer version"). That is
      on purpose.
- [ ] Joiner: an older client can no longer join (protocol 95).

## 4. Pick block (middle click)

- [ ] Creative: middle-click a block. Already in the hotbar: that slot is
      selected; otherwise a full stack goes into your hand (with a full
      hotbar it replaces the selected item).
- [ ] Survival: only works if you carry it. In the hotbar: selects it. In the
      bag: swaps it into the selected hotbar slot. Not carried: nothing.
- [ ] Crops give their seed, a lit furnace gives a furnace, ore gives the ore
      block.
- [ ] Middle-click does nothing while a panel (inventory, chest, chat) is open.
- [ ] As a joiner in creative with a full hotbar, the block lands in the bag.
- [ ] Middle mouse cannot be rebound yet.

Tested commit / platform:

Notes:
