# Full mob roster visual polish — 15 October 2026

User authorised continuing the accepted seven-animal pass across every mob.
The 30 species and Satoshi use the live local/remote entity rendering path.
This is visual only: existing hitboxes, health, speed, AI, saves and wire IDs
remain unchanged. Cosmetic ears, antlers, fins and armour may extend beyond
interaction bounds. No game deployment or main push is included.

## Player checks

- [ ] Cow, pig, sheep and chicken: recognise ears, horns, muzzle, fleece,
  comb/beak and tails; inspect from front, sides and rear.
- [ ] Bear and polar bear: broad muzzle and round ears, brown versus pale fur.
- [ ] Wolf, hyena, horse, rabbit, goat and Nostrich: distinct colours and anatomy;
  inspect the horse mane/blaze, hyena spots, rabbit ears and purple feathers.
- [ ] Bee: joined wide wings, eyes, antennae and six little legs.
- [ ] Squid and glow squid: eight joined arms, eyes and mantle fins in water;
  glow squid stays bright at night.
- [ ] Fish and shark: tails follow behind their direction of movement; fins,
  eyes and shark gills/belly remain readable underwater.
- [ ] Villager and peddler: face the direction they walk; apron versus hood,
  scarf and pack. Existing interactions still open normally.
- [ ] Brigand, marauder, berserker and knight: distinguish hood, grey armour,
  broad red shoulders and steel/blue armour from a distance.
- [ ] Satoshi: visible face and amulet at the front, cloak at the back;
  only the amulet glows in low light. Existing interaction remains available.
- [ ] Walk all groups and watch for detached boots, hands, shoulders or shield.
- [ ] Feed, tame, breed, ride, shear and milk where already supported;
  juveniles scale every detail, and existing saddle/pack interactions work.
- [ ] Host and join a world: both players see matching mob appearance/facing.
- [ ] Workshop: an old body/head/limb reskin still targets the same part;
  paint one new accessory and check the live mob and mannequin agree.
- [ ] Resource pack: unchanged keys load. Donor cow/sheep textures now affect
  only models using them; new materials follow the shared paint and neutral
  surface keys (including sand for warm neutral details).
- [ ] Earlier cat, fox, crab, reindeer, parrot, donkey and mule remain correct.

## Automated evidence

Baseline: 30 production-native PNGs, producer commit18fdd292 and exact commands,
return codes and hashes in `.claude/epic/logs/all-mob-polish/before-renders.json`.
Four new anatomy/identity regressions were introduced before implementation.
First native Clippy/build passed. Independent source review and 86 first native
captures identified and corrected rear-head cheek artefacts, an obscured cow
eye line and a coplanar Nostrich neck junction. Six focused invariants cover
identity, facing, anatomy, attachment transforms, live juvenile/walk rendering,
Workshop reskins and the observed surface defects. Final captures and the
complete regression gate passed. Official `./check.sh` passed all
13 gates: 6534 native tests passed, 0 failed, 124 ignored (2040.53s); strict
Clippy, native build, licences, site/npm consumer checks, WASM, bundle-size and
forbidden-symbol gates passed. The brotli bundle is 4.62 MiB, below 5 MiB. The optional Playwright smoke was not run.

All 86 final production-native captures passed root and independent review,
covering 30 species plus Satoshi; the earlier seven are byte-identical to their
matched baseline. Six focused roster regressions passed on the exact native
test executable. Source snapshot: `655aff006b68`; producer: `0097d169f1fc`.
The owner-private gallery is updated with 100 original PNGs and 38 comparisons.
Exact source, executable, command and image provenance and gate logs are in
`.claude/epic/logs/all-mob-polish/`. These results do not imply manual acceptance.

Live interaction and owner/browser acceptance remain unrun until ticked above.
