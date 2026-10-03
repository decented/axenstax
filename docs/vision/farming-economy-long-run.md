# Farming & Food Economy — Long-Run Master Design

**Status**: VISION DOCUMENT — establishes the end-state. Per-tier foundation specs derive from this.
**Date**: 2026-05-14
**Author commit**: Captures Axolittle's 2026-05-14 design call ("full farming with a strong, varied economy") and frames it across 8 delivery tiers spanning alpha through long-run.
**Companions**: `docs/foundations/2026-05-14-farming-system.md` (Tier 1), `docs/foundations/2026-05-14-farming-tier-1.5-processed-economy.md` (Tier 1.5), Spec 5 (gameplay), Spec 6 (Bitcoin economy). The complementary vision doc covering markets, services, combat, PvP, land, knowledge, and spectator economies — i.e. everywhere farming output **goes** — is `docs/vision/economies-long-run.md`.
**Cross-refs**: Memory axenstax has farming, uk english naming, shared infra strategy, proof of play is proof of work.

---

## 0. TL;DR

AxeNStax farming is **not** a Minecraft clone — it's a **full agricultural economy** with 120+ crops/animal products/processed goods spread across **biome specialization**, **multi-stage processing chains**, and **value laddering up to 5+ tiers of complexity**. The end-state is a world where:

- A player on the temperate plains grows wheat, brews beer from barley+hops, ages cheddar in their cellar.
- A player in the jungle grows cocoa, vanilla, and coffee — and trades them to the plains player for grain.
- A player on the Mediterranean coast presses olives for oil, grows tomatoes and aubergines for pasta sauces.
- A high-tier dish (truffle risotto with aged parmesan, paired with cellar-aged wine) carries genuine economic weight — both as a hunger/saturation/buff payoff *and* as a tradeable item with real **trade-value** that, on Bitcoin-enabled servers, converts to sats.

**Delivery is phased across 8 tiers** (T1 → T8). Each tier ships independently; each leaves the game in a coherent playable state. Tiers 1–2 are alpha/early-content (Minecraft-baseline + automation). Tiers 3–5 expand variety and depth. Tiers 6–8 layer specialization, mastery, and industrial-scale processing.

Built on this foundation: the **shared workstation framework** (Tier 1.5) lifts to other AxeNStax-internal games (diner mechanics, egg incubation, future cooking-focused titles).

---

## 1. Design Principles

These principles guide every farming/food decision. When in doubt, return here.

### 1.1 Variety creates choice; depth creates value

- **Variety** — many crops, animals, ingredients. Players have many options for *what* to do.
- **Depth** — multi-step processing chains. Players have many options for *how far* to take a given ingredient.

Both are required. Variety without depth is a vegetable garden with no cooking. Depth without variety is one chain (wheat → bread → toast) repeated. The strength of the economy comes from both.

### 1.2 Specialization creates trade

If everyone can grow everything everywhere, there's no trade. **Biome specialization** is the engine of player-to-player and server-to-server commerce. The temperate plains grow wheat well but cocoa poorly. The jungle grows cocoa well but wheat poorly. Result: trade happens because it's *mechanically necessary*, not just *socially encouraged*.

### 1.3 Time-investment matters more than skill-investment

The kid-friendly target audience means we don't want twitchy skill checks. But we *do* want time-investment to be meaningful — a 3-day-aged cheese should feel more valuable than a 2-minute-baked bread, because the player chose to invest the time. **Aging Rack and Cellar mechanics** are the primary depth lever, not click-timing or precision tools.

### 1.4 Every crop has at least two roles

Wheat alone wouldn't justify wheat — but wheat → bread → toast → bread pudding is meaningful, *and* wheat → flour → pasta → ravioli with cheese filling is meaningful, *and* wheat → beer (with barley + hops) is meaningful. Every crop earns its slot in the registry by participating in **at least two distinct chains**. Single-role crops get cut or merged.

### 1.5 The Bitcoin layer is opt-in, not load-bearing

Per bitcoin parent controlled and Spec 6, the Bitcoin economy is parent-controlled per-server. **Farming must be fully satisfying without sats payouts.** The trade-value annotation on items always exists; the sats conversion is a server-operator decision. Default servers run trade-value as internal-score-only.

### 1.6 Proof-of-play does NOT apply to cooking

Per proof of play is proof of work, proof-of-play is the mining-side concept (hash on pickaxe strike). Cooking is not mining. The Bitcoin layer for cooking comes via **trade-value → sats conversion** at the item-economy layer, not via hash-on-bake. Don't add hash-driven mechanics to ovens.

### 1.7 UK English throughout

Per uk english naming. Aubergine not eggplant; courgette not zucchini; coriander not cilantro; sugar beet ≠ beetroot.

### 1.8 Cross-game shared infrastructure

The workstation framework, breeding system, biome data, trade-value hook, and recipe-chain pattern are all designed for cross-game lift per shared infra strategy. Recipes themselves are AxeNStax-flavoured; the *machinery* that drives them is generic.

---

## 2. The Tier Roadmap

Eight tiers from baseline farming to industrial-scale economy. Each tier is shippable independently and leaves the game coherent.

| Tier | Theme | Status | Scope summary |
|:---:|---|:---:|---|
| **T1** | **Minecraft-baseline** | SPEC'D — `docs/foundations/2026-05-14-farming-system.md` | Hoe + tilled soil + wheat/carrot/potato + bread. 11 phases, ~1900 lines. |
| **T1.5** | **Processed Economy Base** | SPEC'D — `docs/foundations/2026-05-14-farming-tier-1.5-processed-economy.md` | Workstation framework (Furnace, Mill, Oven, Aging Rack) + sugarcane/sugar beet/beetroot/pumpkin/berries + eggs/milk + 20 new recipes + value ladder + Spec 6 §13 economy hook. 13 phases, ~2500 lines. |
| **T2** | **Animal-drawn Automation** | SKETCHED — bottom of T1 spec | Plough + 4 draft animals (cow/horse/donkey/mule, mule bred-only) + breeding system + fences + mounted-or-fenced-autonomy work modes. Own foundation spec post-T1 playtest. |
| **T3** | **Orchards + Drinks** | THIS DOC FRAMES IT | Tree fruit (apple, pear, plum, cherry, fig, citrus), Brewing Vat + cider/wine/mead/beer/ale, Juicing Press, expanded berry set, Hops + Barley as drink-source crops. |
| **T4** | **Preservation + Variety Expansion** | THIS DOC FRAMES IT | Drying Rack, Smokehouse, Salting Box, Pickling Jars. Expanded crop set (tomato, pepper, cabbage, lettuce, peas, beans, onion, garlic, leek, herbs). Cured meats, preserved foods, sauerkraut, kimchi, pickles. |
| **T5** | **Biome Specialization + Exotic Crops** | THIS DOC FRAMES IT | Climate-gated crops. Jungle (cocoa, vanilla, banana, coffee, ginger, pineapple). Mediterranean (olive, tomato proper, citrus). Tundra (hardy roots, lingonberry). Desert (date palm). Wetland (rice, lotus, cranberry, water chestnut). Tea cultivation. |
| **T6** | **Apiculture + Maple + Specialty Animals** | THIS DOC FRAMES IT | Bees + Hives + Honey + Beeswax. Maple trees + tapping mechanic + syrup. Goats (goat milk → harder cheeses), Ducks/Geese/Turkeys. Rabbits. Specialty cheese cellars. |
| **T7** | **Buff Foods + Mastery + Quality Grades** | THIS DOC FRAMES IT | Quality grades on harvest (common/fine/exquisite). Buff system — high-tier foods grant temporary effects (speed, jump, regen, night vision, water breathing, fortune). Masterclass recipes. Heirloom seed variants. |
| **T8** | **Industrial / Automated** | THIS DOC FRAMES IT | Greenhouses (climate-controlled growing). Hydroponics. Industrial Mill / Power Oven (batch processing). Conveyor-fed workstations. Maybe steam/water/wind power for free fuel. |

**Estimated delivery time** (very rough, gates on Axolittle playtest between tiers):
- T1: 2-3 weeks (current ready-to-build)
- T1.5: 4-6 weeks
- T2: 4-6 weeks
- T3: 3-4 weeks
- T4: 4-5 weeks
- T5: 6-8 weeks (worldgen integration is the slowdown)
- T6: 3-4 weeks
- T7: 4-5 weeks
- T8: 6-10 weeks

Total: roughly 8-12 months of farming-content work, paced by Axolittle's playtest feedback between tiers.

---

## 3. Master Crop Catalogue

The full long-run crop registry. **80 entries** spanning grains, roots, brassicas, legumes, cucurbits, nightshades, alliums, herbs, sugars, drinks-sources, berries, tree fruit, nuts, mushrooms, exotic specialties, and industrial (non-food) plants.

Each entry: name, primary biome, introduced-in-tier, primary uses, growth notes.

### 3.1 Grains (staples — the foundation of bread, brewing, animal feed)

| Crop | Biome | Tier | Primary uses | Growth |
|---|---|:---:|---|---|
| **Wheat** | Temperate plains | T1 | Flour → bread/cake/pasta/pies; pig feed | 4 stages, ~30s |
| **Barley** | Temperate plains | T3 | Beer brewing; malt; livestock feed; hardier than wheat (grows colder) | 4 stages, ~30s |
| **Oats** | Temperate / cool highland | T4 | Porridge; oatcakes; horse feed; haggis filling | 4 stages, ~35s |
| **Rye** | Cool / hardy soil | T4 | Dark bread (rye bread, pumpernickel); whisky base; bread variant | 4 stages, ~35s |
| **Maize / Corn** | Warm temperate | T4 | Cornmeal; tortillas; corn-on-the-cob; popcorn; livestock feed | 4 stages, ~40s, tall (2 blocks) |
| **Rice** | Wetland / paddy | T5 | Steamed rice; risotto; rice pudding; sake; sushi (with fish) | Special: requires flooded tile, 4 stages |
| **Spelt** | Cool temperate | T7 (heirloom) | Specialty bread; higher value than wheat; pre-modern grain | 4 stages |
| **Buckwheat** | Cool / poor soil | T7 (heirloom) | Pancakes; soba noodles; gluten-free flour | 4 stages |
| **Quinoa** | Cool highland | T5 | Pseudo-grain; salads; high-protein bowls; rice alternative | 4 stages |

### 3.2 Root vegetables (carb / sugar / aromatic backbone)

| Crop | Biome | Tier | Primary uses | Growth |
|---|---|:---:|---|---|
| **Carrot** | Temperate | T1 | Raw; soup; stew; cake; horse feed | 4 stages, ~30s |
| **Potato** | Temperate / cool | T1 | Baked; chips; mash; stew; vodka base | 4 stages, ~30s |
| **Sugar Beet** | Cool temperate | T1.5 | Mill → Sugar (lower yield than cane) | 4 stages, ~30s |
| **Beetroot** | Cool temperate | T1.5 | Raw (low); Beetroot Soup; pickling; salad | 4 stages, ~30s |
| **Turnip** | Cool / hardy | T4 | Stews; mash; pickling; tundra-viable | 4 stages, ~25s |
| **Parsnip** | Cool temperate | T4 | Roasted; soup; sweet root | 4 stages, ~35s |
| **Swede (Rutabaga)** | Cool | T4 | Mash (haggis side); stew; tundra-viable | 4 stages, ~35s |
| **Sweet Potato** | Warm | T5 | Baked; pie; chips; jungle-edge crop | 4 stages, ~40s |
| **Radish** | Temperate | T4 | Quick crop (fast growth); raw; pickled | 4 stages, ~10s (fastest crop) |
| **Garlic** | Mediterranean / temperate | T4 | Universal ingredient; antimicrobial buff (T7)? | 4 stages, ~40s |
| **Onion** | Temperate | T4 | Ubiquitous in cooked recipes; pickled | 4 stages, ~30s |
| **Leek** | Cool temperate | T4 | Soups (leek-and-potato); stews; specialty | 4 stages, ~35s |
| **Ginger** | Jungle | T5 | Tea; baking spice; ginger beer; medicinal-buff | 4 stages, ~40s, jungle-only |
| **Horseradish** | Cool temperate | T7 (heirloom) | Specialty condiment; high trade-value | 4 stages, slow |
| **Cassava (Yuca)** | Jungle | T5 | Boiled / fried staple; tapioca pearls (pudding, bubble tea); jungle carb backbone | 4 stages, slow |
| **Yam** | Tropical | T5 | Roasted; fufu; pounded yam; distinct from Sweet Potato (different botanical family) | 4 stages, slow |

### 3.3 Brassicas & leafy greens (vegetable variety, vitamin C)

| Crop | Biome | Tier | Primary uses | Growth |
|---|---|:---:|---|---|
| **Cabbage** | Cool temperate | T4 | Sauerkraut; kimchi; coleslaw; stew | 4 stages, ~35s |
| **Lettuce** | Temperate | T4 | Salads; sandwich filler; quick-growing | 4 stages, ~15s |
| **Kale** | Cool / hardy | T4 | Salads; chips; smoothies; hardier than lettuce | 4 stages, ~30s |
| **Spinach** | Temperate | T4 | Salads; quiche filling; soup | 4 stages, ~20s |
| **Brussels Sprouts** | Cool | T7 | Christmas dinner association; specialty | 4 stages, slow |
| **Cauliflower** | Temperate | T4 | Cheese sauce dish; pickled (piccalilli); curry | 4 stages, ~40s |
| **Broccoli** | Temperate | T4 | Stir-fry; cheese sauce; healthy buff | 4 stages, ~35s |
| **Pak Choi (Bok Choy)** | Warm temperate | T5 | Stir-fry pillar; light steamed; pairs with soy + ginger | 4 stages, ~25s |
| **Watercress** | Wetland edge / cool stream | T5 | UK salad staple; soup; sandwich filler; wetland-grown leafy green | 4 stages, ~20s, needs water-adjacency |

### 3.4 Legumes (protein source, soil-improving)

| Crop | Biome | Tier | Primary uses | Growth |
|---|---|:---:|---|---|
| **Peas** | Temperate | T4 | Soup; mushy peas; freezable | 4 stages, ~30s |
| **Broad Beans** | Temperate | T4 | Stews; falafel-ish; protein | 4 stages, ~30s |
| **Runner Beans** | Temperate | T4 | Pickling; stir-fry; tall stem (climbing) | 4 stages, ~35s |
| **Kidney Beans** | Warm temperate | T4 | Chilli; stews; dried-store | 4 stages, ~40s |
| **Lentils** | Mediterranean | T5 | Dal; soup; high-protein | 4 stages |
| **Chickpeas** | Mediterranean | T5 | Hummus; falafel; curry | 4 stages |
| **Soybeans** | Warm temperate | T5 | Tofu; soy sauce; soy milk | 4 stages |
| **Peanut (Groundnut)** | Warm temperate | T5 | Roasted snack; peanut butter; satay sauce; peanut oil (lubricant chain — §10.7) | 4 stages, pods grow underground |
| **Green Beans (French Beans)** | Temperate | T4 | Steamed side; stir-fry; salade niçoise; pod-eaten, distinct from dried Runner Beans | 4 stages, ~30s |

### 3.5 Cucurbits & squashes (versatile, often decorative)

| Crop | Biome | Tier | Primary uses | Growth |
|---|---|:---:|---|---|
| **Pumpkin** | Temperate | T1.5 | Pie; soup; carving (decoration); seeds | Stem-grown, 4 stem stages + block |
| **Courgette (Marrow)** | Warm temperate | T4 | Ratatouille; bread; soup; quick-growing | Stem-grown, smaller |
| **Butternut Squash** | Warm temperate | T4 | Roasted; soup; pasta filling | Stem-grown |
| **Watermelon** | Warm / desert-edge | T5 | Hydration; sweet refreshment; rare | Stem-grown, slow |
| **Cantaloupe / Melon** | Warm | T5 | Fresh fruit; salads; rare | Stem-grown, slow |
| **Cucumber** | Warm temperate | T4 | Salad; pickled (gherkin); raita | Vine-grown |

### 3.6 Nightshades & fruiting vegetables (the Italian/Mediterranean pillar)

| Crop | Biome | Tier | Primary uses | Growth |
|---|---|:---:|---|---|
| **Tomato** | Mediterranean / warm temperate | T4 | Sauce (huge chain); raw; sun-dried; ketchup; soup | Stem-grown, 5 stages |
| **Sweet Pepper** | Warm temperate | T4 | Stuffed pepper; salad; ratatouille; paprika (dried) | Stem-grown, 5 stages |
| **Chilli Pepper** | Warm temperate | T4 | Spice; chilli sauce; chilli con carne; buff effect (T7) | Stem-grown, 5 stages |
| **Aubergine** | Mediterranean | T5 | Moussaka; ratatouille; baba ganoush; parmigiana | Stem-grown, 5 stages |
| **Tomatillo** | Mediterranean / warm | T7 (heirloom) | Salsa verde; speciality recipes | Stem-grown |

### 3.7 Herbs & aromatics (flavour multipliers — small inputs, big effect)

| Crop | Biome | Tier | Primary uses | Growth |
|---|---|:---:|---|---|
| **Basil** | Mediterranean | T4 | Pesto; pasta; pizza topping | 3 stages |
| **Mint** | Temperate (invasive!) | T4 | Tea; lamb sauce; mojito | 3 stages, spreads |
| **Parsley** | Temperate | T4 | Garnish; sauces; stuffing | 3 stages |
| **Coriander** | Warm temperate | T4 | Curry; salsa; UK spelling, not "cilantro" | 3 stages |
| **Rosemary** | Mediterranean | T4 | Lamb roast; bread; oil-infusion | Bush, perennial |
| **Thyme** | Mediterranean | T4 | Stews; chicken; bread | Bush, perennial |
| **Sage** | Mediterranean | T4 | Stuffing; sage-and-onion; butter sauce | Bush, perennial |
| **Oregano** | Mediterranean | T4 | Pizza; pasta sauce | Bush, perennial |
| **Dill** | Temperate / cool | T4 | Pickling; fish accompaniment; soup | 3 stages |
| **Chives** | Temperate | T4 | Garnish; potato dishes; mild allium | 3 stages, perennial |
| **Lavender** | Mediterranean | T5 | Specialty baking; aromatic; decorative | Bush, perennial |
| **Bay Laurel** | Mediterranean | T5 | Stews; bouquet garni; tree | Tree, slow |
| **Fennel** | Mediterranean | T4 | Dual veg/herb — bulb roasted, fronds garnish; fish dishes; sausage; aniseed flavour | 4 stages, bulb-and-frond |

### 3.8 Sugars & sweeteners

| Source | Biome | Tier | Primary uses | Growth |
|---|---|:---:|---|---|
| **Sugarcane** | Warm / wetland-adjacent | T1.5 | Mill → Sugar (1:1); high yield | Vertical stack, water-adjacent |
| **Sugar Beet** | Cool temperate | T1.5 | Mill → Sugar (3:1); backup sugar source | (see roots) |
| **Honey** | Anywhere bees nest | T6 | Sweetener (premium); mead base; medicine buffs | (see apiculture) |
| **Maple Syrup** | Cool forest | T6 | Sweetener (premium); pancake topping; specialty | Tree-tapped, slow |
| **Stevia** | Mediterranean / herb | T7 | No-calorie sweetener; specialty | 3 stages |

### 3.9 Drink-source crops (separate from food — drinks have their own value chain)

| Crop | Biome | Tier | Primary uses | Growth |
|---|---|:---:|---|---|
| **Hops** | Cool temperate | T3 | Beer brewing (bittering agent); essential brewing | Climbing vine, perennial, tall |
| **Tea Bush** | Cool highland / mountain | T5 | Black/green/herbal tea; leaves harvest | Bush, perennial, slow |
| **Coffee Plant** | Jungle / tropical | T5 | Coffee beans → roasted → ground; premium drink | Bush, perennial, very slow |
| **Grape Vine** | Mediterranean | T3 | Wine (huge value chain); raisins; juice | Vine, perennial, multi-year |
| **Apple Tree** | Temperate | T3 | Cider; pie; juice; fresh | Tree, perennial (orchards) |
| **Cocoa Tree** | Jungle | T5 | Cocoa beans → chocolate (premium chain) | Tree, jungle-only, slow |
| **Vanilla Orchid** | Jungle | T5 | Vanilla pods (premium spice) | Orchid (vine on tree), very rare/slow |

### 3.10 Berries & bush fruit (replenishing harvests, no replant)

| Crop | Biome | Tier | Primary uses | Growth |
|---|---|:---:|---|---|
| **Mixed Berries** | Temperate forest edge | T1.5 | Jam; pie; raw; cordial | Bush, right-click harvest |
| **Strawberry** | Temperate | T3 | Jam; cream; cake topping; raw | Low plant, runners |
| **Raspberry** | Cool temperate | T3 | Jam; raw; pie; raspberry vinegar | Bush, perennial |
| **Blackberry** | Temperate hedge | T3 | Jam; pie; foraging-viable | Bramble (thorny — slight damage!) |
| **Blueberry** | Cool / acidic soil | T3 | Muffins; pancakes; raw; jam | Bush, perennial |
| **Gooseberry** | Cool temperate | T3 | Fool; crumble; jam; sharp flavour | Bush, perennial |
| **Blackcurrant** | Cool temperate | T7 | Cordial; jam; cassis; UK specialty | Bush, perennial |
| **Redcurrant** | Cool temperate | T7 | Jelly; specialty | Bush, perennial |
| **Lingonberry** | Tundra | T5 | Jam; tundra-only specialty; high trade-value | Bush, very cold |
| **Cranberry** | Wetland | T5 | Sauce; juice; bog-grown specialty | Wetland, very slow |
| **Elderberry** | Temperate hedgerow | T3 | Elderflower cordial (from flowers); elderberry wine; jam; UK hedgerow specialty | Tree-bush, perennial |

### 3.11 Tree fruit (orchards — multi-year payoff)

| Crop | Biome | Tier | Primary uses | Growth |
|---|---|:---:|---|---|
| **Apple** | Temperate | T3 | Pie; cider; juice; fresh; dried | Tree, ~3 in-game seasons to mature, annual yields |
| **Pear** | Temperate | T3 | Pie; poached; perry (drink); fresh | Tree, similar to apple |
| **Plum** | Temperate | T3 | Jam; brandy; fresh; dried (prune) | Tree |
| **Cherry** | Temperate | T3 | Pie; kirsch (drink); fresh; preserved | Tree |
| **Peach** | Warm temperate | T5 | Cobbler; fresh; jam; nectar | Tree |
| **Apricot** | Warm temperate | T5 | Jam; dried; tart; specialty | Tree |
| **Fig** | Mediterranean | T5 | Fresh; dried; specialty jam | Tree, slow |
| **Lemon** | Mediterranean / warm | T5 | Cooking acid; lemonade; baking | Tree, citrus |
| **Orange** | Mediterranean / warm | T5 | Juice; marmalade; baking | Tree, citrus |
| **Lime** | Tropical | T5 | Cooking; cocktails | Tree, citrus |
| **Banana** | Tropical | T5 | Fresh; bread; smoothies | "Tree" (large stem), tropical |
| **Coconut** | Tropical | T5 | Milk; oil; meat; tropical specialty | Palm tree |
| **Olive** | Mediterranean | T5 | Oil (huge value chain); table olives; cured | Tree, very slow, perennial |
| **Avocado** | Warm subtropical | T7 | Fresh; guacamole; specialty | Tree |
| **Date Palm** | Desert / oasis | T5 | Dates (dried sweet fruit); desert specialty | Palm tree, oasis-only |
| **Pomegranate** | Mediterranean | T7 | Seeds; juice; specialty | Tree |

### 3.12 Nuts (long-term tree crops)

| Crop | Biome | Tier | Primary uses | Growth |
|---|---|:---:|---|---|
| **Hazelnut** | Cool temperate | T7 | Praline; cake; chocolate hazelnut spread | Bush/small tree |
| **Walnut** | Temperate | T7 | Cake; oil; pickled (green); brain-shaped | Tree, large, slow |
| **Almond** | Mediterranean | T7 | Marzipan; milk; cake; specialty | Tree |
| **Chestnut** | Temperate woodland | T7 | Roasted; stuffing; flour; specialty | Tree, very large |
| **Pistachio** | Mediterranean | T7 | Snack; cake; ice cream | Tree |
| **Cashew** | Tropical | T7 | Snack; vegan cheese base; korma curry; chocolate inclusion; cashew oil | Tree, tropical, slow |
| **Pecan** | Warm temperate | T7 | Pecan pie; praline; pecan oil; North American specialty | Tree, slow |

### 3.13 Mushrooms & fungi (forage / cultivated)

| Type | Biome | Tier | Primary uses | Growth |
|---|---|:---:|---|---|
| **Brown Mushroom** | Forest / dark | T4 (worldgen spawn) | Soup; stew; raw | Spreads on grass (when worldgen wires it; currently absent from code) |
| **Red Mushroom** | Forest / dark | T4 (worldgen spawn) | Decoration; not eaten | Spreads on grass |
| **Cave Mushroom** | Deep cave | T4 (worldgen spawn) | Soup ingredient | Cave-floor spawn |
| **Cultivated Mushroom** | Dark + damp block (any darkness-level-suitable substrate; no dedicated workstation) | T7 | Pizza topping; risotto; mass production | Spread mechanism inside any dark room with mycelium substrate |
| **Truffle** | Rare forest, pig-foraged | T7 | Risotto; pasta; premium ingredient; pig-required! | Pig-foraged only |
| **Morel** | Forest spring | T7 | Specialty cooking; rare seasonal | Forest, seasonal |
| **Porcini** | Pine forest | T7 | Risotto; dried specialty; Italian | Forest, seasonal |

### 3.14 Exotic & specialty plants

| Crop | Biome | Tier | Primary uses | Growth |
|---|---|:---:|---|---|
| **Pineapple** | Tropical | T5 | Fresh; juice; pizza (controversial!); tropical specialty | Plant-grown |
| **Mango** | Tropical | T5 | Fresh; chutney; smoothies | Tree |
| **Papaya** | Tropical | T5 | Fresh; meat tenderiser; smoothies | Tree |
| **Pomelo** | Tropical | T7 | Specialty citrus | Tree |
| **Dragon Fruit** | Cactus / arid | T7 | Specialty fruit; striking visual | Cactus |
| **Kiwi** | Cool temperate | T7 | Fresh; specialty | Vine |
| **Saffron** | Mediterranean / arid | T7 | World's most expensive spice; tiny yield; massive trade-value | Crocus, very slow |

### 3.15 Industrial & non-food (clothes, dyes, materials)

| Crop | Biome | Tier | Primary uses | Growth |
|---|---|:---:|---|---|
| **Cotton** | Warm temperate | T8 | Cloth (replaces wool); textile chain | 4 stages |
| **Flax** | Cool temperate | T8 | Linen cloth; oil (linseed) | 4 stages |
| **Hemp** | Temperate | T8 | Rope; cloth; durable fibre | 4 stages, tall |
| **Indigo** | Warm | T8 | Blue dye; textile dyeing | 4 stages |
| **Madder** | Mediterranean | T8 | Red dye | Bush |
| **Woad** | Cool temperate | T8 | Blue dye (UK historical) | 4 stages |
| **Tobacco** | Warm | (Excluded by design) | (Not appropriate for a kid-target game) | — |
| **Sunflower** | Warm temperate | T7 | Oil (press); seeds (snack); decoration | Tall plant |
| **Rapeseed (Canola)** | Cool temperate | T7 | Oil press; commodity oil | 4 stages |
| **Lavender** | (see herbs) | T5 | (also industrial — perfume, soap) | (see herbs) |

**Total catalogue: ~135 distinct entries** (15 categories above; Sugar Beet and Lavender are listed twice because they participate in multiple roles, so unique-name count is ~133). Not every entry needs its own block ID — tree fruit share a single "Fruit Tree" block type with crop discriminator, mushrooms reuse a single mushroom-block, and herbs may share a "Herb Bush" block. Realistic **crop-block-ID budget: ~85**, with the remaining variety expressed via item-only or shared-block-with-data. Worldgen + texture-gen + UI layout scope against the ~85 block-ID figure; item textures + recipe table scope against the ~135 catalogue figure.

---

## 4. Master Animal Product Catalogue

### 4.1 Existing animals (T1 — already in code)

- **Cow** → beef (raw + cooked) → milk (T1.5) → leather
- **Pig** → pork (raw + cooked)
- **Chicken** → chicken meat (raw + cooked) → eggs (T1.5) → feathers
- **Sheep** → mutton (raw + cooked) → wool

### 4.2 T2 farming-enabler animals

- **Horse** — draft animal (T2 plough work)
- **Donkey** — draft animal (half-speed)
- **Mule** — bred-only Donkey × Horse (T2)

### 4.3 T6 specialty animals (apiculture + dairy variants + niche meat)

- **Goat** → goat meat → goat milk → goat cheese (specialty)
- **Duck** → duck meat → duck eggs (larger than chicken) → duck down (premium pillow stuffing if textiles exist)
- **Goose** → goose meat → goose eggs (very large) → goose fat (premium cooking fat) → down
- **Turkey** → turkey meat (large, festive)
- **Rabbit** → rabbit meat → rabbit fur (specialty cloth additive)
- **Bee** (technically not livestock but parallel system) → honey → beeswax → propolis (medicinal buff)

### 4.4 T8 industrial / aquatic (long-run)

- **Fish** (separate fishing system, parallel to farming):
  - Cod, salmon, trout, mackerel, tuna, sardines
  - Shellfish (cultivated): mussels, oysters, prawns
  - Smoked salmon, salt cod, anchovies (preservation chain)
- **Snail** (escargot specialty — niche)
- **Frog** (frog legs — niche)

### 4.5 Animal-derived products summary

| Product | Source | Tier | Uses |
|---|---|:---:|---|
| Egg (chicken) | Chicken laying | T1.5 | Baking, cooking |
| Duck Egg | Duck laying | T6 | Premium baking |
| Goose Egg | Goose laying | T6 | Premium specialty baking (large) |
| Milk (cow) | Cow milking | T1.5 | Cheese, butter, cream, drinks |
| Goat Milk | Goat milking | T6 | Specialty cheese (feta, chèvre) |
| Sheep Milk | Sheep milking | T7 | Specialty cheese (pecorino, manchego) |
| Cream | Mill of milk | T1.5 | Butter, cheese, cooking |
| Butter | Aging Rack of cream | T1.5 | Cooking, baking, premium |
| Cheese (basic) | Aging Rack 3-day | T1.5 | Foods, melting, premium |
| Cheddar | Cellar 30-day | T7 | Premium aged cheese (high trade-value) |
| Brie | Cellar 14-day | T7 | Specialty soft cheese |
| Parmesan | Cellar 90-day | T7 | Top-tier hard cheese (huge trade-value) |
| Feta (sheep/goat) | Brine-cure | T7 | Mediterranean specialty |
| Cow-Milk Yoghurt | Aging Rack ¼-day | T3 | Light food, base for tzatziki, dressings |
| Goat Yoghurt | Aging Rack ¼-day (goat milk) | T6 | Tangy specialty yoghurt |
| Labneh | Aging Rack ½-day (sheep milk) | T6 | Mediterranean strained yoghurt |
| Honey | Beehive | T6 | Sweetener, mead, medicine buff |
| Beeswax | Beehive | T6 | Candles, food coating, preservation |
| Maple Syrup | Maple tap | T6 | Premium sweetener, pancake topping |
| Leather | Cow / pig | T1 | Armour, books, premium items |
| Wool | Sheep | T1 | Cloth, beds, decoration |
| Feathers | Chicken / duck / goose | T1 / T6 | Arrows, pillows, decoration |

---

## 5. Master Workstation Catalogue

The processing stations that turn raw into refined. Each is a block-entity on the shared workstation framework (delivered T1.5 Phase 2).

| Station | Tier | Purpose | Slots | Time model | Fuel? |
|---|:---:|---|---|---|:---:|
| **Furnace** | T1.5 | Smelting + simple cooking (cooked meats, baked potato, glass) | 1 in + 1 fuel + 1 out | Ticks (200 default) | Yes |
| **Mill** | T1.5 | Grinding (wheat→flour, sugarcane→sugar) | 1 in + 1 out | Ticks (100 default) | No |
| **Oven** | T1.5 | Multi-ingredient baking (cake, bread, pies, cookies) | 3 in + 1 fuel + 1 out | Ticks (200-300) | Yes |
| **Aging Rack** | T1.5 | Slow-time recipes (cream→butter/cheese, yoghurt) | 1 in + 1 out | In-game days | No |
| **Cooking Pot** | T3 | Stews, soups, broths (multi-veg + liquid) | 4 in + 1 fuel + 1 out (liquid) | Ticks | Yes |
| **Brewing Vat** | T3 | Beer, wine, mead, cider (long fermentation) | 3 in + 1 out (liquid) | In-game days | No |
| **Juicing Press** | T3 | Apple → cider base; grape → must; olive → oil | 1 in + 1 out (liquid) | Ticks (50) | No |
| **Drying Rack** | T4 | Sun-dried tomatoes; raisins; jerky; dried herbs | 4 in + 4 out | In-game days (1-3) | No |
| **Smokehouse** | T4 | Smoked meat, smoked fish, smoked cheese | 4 in + 1 fuel (wood) + 4 out | In-game hours (sub-day) | Yes (wood, not coal) |
| **Salting Box** | T4 | Cured meats, salt cod, salt-cured fish | 1 in + 1 in (salt) + 1 out | In-game days | No |
| **Pickling Jar** | T4 | Pickled veg, sauerkraut, kimchi, gherkins | 1 in + 1 in (vinegar/brine) + 1 out | In-game days | No |
| **Oil Press** | T5 | Olive → oil; sunflower → oil; sesame → oil | 1 in + 1 out (liquid) | Ticks (100) | No |
| **Cheese Cave** | T7 | Premium aged cheeses (cheddar 30-day, parmesan 90-day) | 4 in + 4 out | In-game weeks/months | No (climate-controlled — needs cool biome OR underground placement) |
| **Distillery** | T7 | Spirits (whisky, brandy, gin, vodka) | 1 in (fermented base) + 1 fuel + 1 out (spirit) | Ticks + days | Yes |
| **Beehive (managed)** | T6 | Honey + beeswax extraction (passive over time) | 0 in + 2 out (honey, wax) | Continuous, time-based | No |
| **Greenhouse** | T8 | Climate-controlled growing (off-biome crops) | (grow space, not slots) | Real growth, faster | Glass + materials |
| **Hydroponics Tank** | T8 | Water-only growing (no soil) | (grow space) | Real growth, fastest | Water + power |
| **Industrial Mill** | T8 | Batch mill (10× speed; multiple input) | 8 in + 8 out | Ticks (fast) | Power (steam/water/wind) |
| **Power Oven** | T8 | Batch oven (10× speed) | 8 in + 8 out | Ticks (fast) | Power |
| **Composter** | T7 | Food waste / plant trimmings → Compost / Fertilizer | 4 in + 1 out | In-game days | No |
| **Maple Tap** | T6 | Maple tree → sap → syrup (boiled in Cooking Pot) | (attached to tree, not standalone) | Continuous, seasonal | No |

**21 workstation entries** across the long-run roadmap, but **not all 21 are mechanically distinct**:
- **Industrial Mill (T8) is the batch / power upgrade of Mill (T1.5)** — same recipes, different speed + scale. Same block-entity kind with a `tier: Manual | Industrial` discriminator.
- **Power Oven (T8) is the batch / power upgrade of Oven (T1.5)** — same pattern.
- **Oil Press (T5) and Juicing Press (T3)** are mechanically identical (input → liquid output, no fuel) and may collapse into a single "Press" station with recipe-table-driven inputs.

True mechanical-kind count: **~17 distinct workstation kinds**. Each follows the shared framework — adding a new kind is a recipe table + a block ID + a UI parameter set, not a framework change. Maple Tap is the one exception: it's tree-attached, not a standalone block-entity.

---

## 6. Recipe Chain Depth Examples

The economy's depth comes from chains. Below are 12 representative chains showing how raw inputs ladder to high-value finished goods. Not exhaustive — illustrates the pattern. Tier annotations indicate the first complete-chain availability.

### 6.1 Wheat → Bread family

```
Wheat ─(Mill)→ Flour ─(Grid: +Egg +MilkBucket)→ Dough ┬─(Oven)→ Bread                      [T1.5]
                                                     ├─(Oven +Sugar)→ Sweet Bread          [T1.5]
                                                     ├─(Oven +Sugar +Egg +Milk)→ Cake      [T1.5]
                                                     ├─(Oven +Sugar +Berries)→ Berry Pie   [T1.5]
                                                     └─(Oven +Sugar +Pumpkin)→ Pumpkin Pie [T1.5]

Flour ─(Grid: +Egg +Milk)→ Pasta Dough ─(Pasta Roller, T7)→ Pasta Sheets                 [T7]
                                                            ├→ Spaghetti                  [T7]
                                                            ├→ Lasagne                    [T7]  (with Tomato Sauce + Cheese, Oven)
                                                            └→ Ravioli                    [T7]  (filled with Cheese / Spinach)

Flour + Yeast + Water + Time(Aging Rack) → Sourdough Starter → Artisan Bread             [T7]
```

**Note on Pasta Roller**: Pasta-based dishes require a new T7 workstation (Pasta Roller — extruder-style). Until T7 lands, Italian-style pasta dishes are unavailable; flat-baked alternatives (Pizza-style on Dough) cover the Tomato-Sauce chain at T5.

### 6.2 Tomato → Italian cuisine pillar (T4-T7)

```
Tomato ─(Cooking Pot +Garlic +Onion +Basil +Oregano)→ Tomato Sauce                        [T4]
       │
       ├─(Drying Rack, sun-dried)→ Sun-Dried Tomatoes (specialty ingredient)               [T4]
       │
       └─(Furnace +Bowl)→ Tomato Soup                                                       [T4]

Tomato Sauce ┬─(Oven +Dough +Cheese)→ Pizza Margherita (with Basil)                       [T5]
             ├─(Oven +Pasta +Cheese +Beef)→ Lasagne                                       [T7]  ← needs Pasta Roller
             ├─(Cooking Pot +Pasta)→ Spaghetti Bolognese (with Beef)                      [T7]  ← needs Pasta Roller
             └─(Cooking Pot +Aubergine +Courgette +Pepper)→ Ratatouille                   [T5]
```

### 6.3 Milk → Dairy ladder

```
MilkBucket ─(Aging Rack ½-day)→ Cream + empty Bucket                                      [T1.5]
                              │
                              ├─(Aging Rack 1-day)→ Butter                                [T1.5]
                              ├─(Aging Rack 3-day)→ Cheese (basic, cow-milk)              [T1.5]
                              └─(Aging Rack ¼-day)→ Cow-Milk Yoghurt                     [T3 — Aging Rack ships T1.5 but yoghurt recipe + texture lands T3]

T6 adds goat milk + sheep milk, opening yoghurt + cheese variants:
                              ├─(GoatMilkBucket + Aging Rack ¼-day)→ Goat Yoghurt        [T6]
                              └─(SheepMilkBucket + Aging Rack ½-day)→ Labneh             [T6]

Cheese (basic) is the entry tier. T7 unlocks the **Cheese Cave** for premium aged:
  ├─(Cheese Cave 30-day, cow milk)→ Cheddar                                               [T7]
  ├─(Cheese Cave 14-day, cow milk)→ Brie
  ├─(Cheese Cave 90-day, cow milk)→ Parmesan                                              [T7]  ← top tier
  ├─(Salting Box + Sheep Milk)→ Feta                                                       [T7]
  ├─(Goat Milk + Aging Rack)→ Chèvre                                                       [T7]
  └─(Cheese Cave + Goat Milk + 60-day)→ Aged Goat Cheese                                  [T7]
```

### 6.4 Apple → Cider family (T3)

```
Apple ─(Juicing Press)→ Apple Juice ─(Brewing Vat 3-day)→ Cider (mild alcohol, T3 age-gated)
                                                         └─(Brewing Vat 14-day +sugar)→ Cider Brandy [T7 Distillery]

Apple ─(Furnace +Sugar)→ Baked Apple                                                      [T3]

Apple ─(Drying Rack 2-day)→ Apple Rings                                                   [T4]

Apple +Sugar +Dough ─(Oven)→ Apple Pie                                                    [T3]
  (Cinnamon optional buff variant lands T5 once cinnamon is in the spice catalogue)

Apple +Sugar ─(Cooking Pot 1-day)→ Apple Sauce
                                  └→ Apple Butter (longer reduction)                      [T4]
```

### 6.5 Grape → Wine family (T3-T7)

```
Grape ─(Juicing Press)→ Grape Must ─(Brewing Vat 7-day)→ Young Wine
                                                          └─(Cellar 30-day)→ Wine          [T3-T7]
                                                                            └─(Cellar 365-day)→ Vintage Wine [T7 — top trade-value]

Wine ─(Distillery)→ Brandy                                                                 [T7]

Grape ─(Drying Rack 5-day)→ Raisins                                                       [T4]
```

### 6.6 Hops + Barley → Beer family (T3)

```
Barley ─(Furnace, low-heat)→ Malted Barley ─(Brewing Vat +Hops +Water 3-day)→ Beer        [T3]
                                                                              └─(Cellar 14-day)→ Ale
                                                                              └─(longer ferment +rye)→ Stout
```

### 6.7 Olive → Oil chain (T5)

```
Olive ─(Oil Press)→ Olive Oil ┬─→ Cooking ingredient (universal in Mediterranean recipes)
                              ├─(infuse Rosemary)→ Rosemary-Infused Oil                   [T5+]
                              ├─(infuse Chilli)→ Chilli Oil
                              └─(applied to cart / machine)→ Lubricant — see §10.7        [T2+ carts; T8 industrial]

Olive ─(Salting Box +herbs)→ Cured Olives (snack, salad ingredient)                       [T5]
```

### 6.8 Cocoa → Chocolate (T5-T7)

```
Cocoa Pod (from Cocoa Tree, jungle) ─(harvest)→ Cocoa Beans ─(Furnace dry)→ Roasted Beans
                                                                            └─(Mill)→ Cocoa Powder
Cocoa Powder ─(Grid +Sugar +Milk)→ Hot Chocolate (drink, warming buff in cold biomes)     [T5]
Cocoa Powder + Sugar + Milk + Butter ─(Aging Rack 1-day, cool temperature)→ Chocolate Bar [T7]
Chocolate Bar ─(Grid +Hazelnut)→ Hazelnut Chocolate                                       [T7]
Chocolate Bar ─(Grid +Mint)→ Mint Chocolate                                               [T7]
```

### 6.9 Coffee → Espresso (T5-T7)

```
Coffee Cherry (tropical) ─(harvest)→ Coffee Beans (green) ─(Furnace, light roast)→ Roasted Beans
                                                                                     └─(Mill)→ Ground Coffee

Ground Coffee +Water ─(Cooking Pot)→ Coffee (drink, alertness buff)                       [T5]
                                  └─(special: Espresso Machine T8)→ Espresso (stronger)   [T8]
```

### 6.10 Honey → Mead family (T6)

```
Honey ─(Brewing Vat +Water 7-day)→ Mead                                                   [T6]
       └─(Cellar 30-day)→ Aged Mead

Honey ─(Grid +Bread)→ Honey Toast (snack)

Honey ─(Grid +Tea Leaves +Water)→ Honey Tea (warming, mild healing)                       [T6]

Beeswax ─(Grid +Wool wick)→ Candle (decoration + light source)                            [T6]
        └─(Grid +Hard Cheese top)→ Wax-Sealed Cheese (preservation)                       [T7]
```

### 6.11 Risotto / Rice chain (T5-T7)

```
Rice (paddy-grown) ─(Cooking Pot +Water)→ Plain Cooked Rice (low value)                   [T5]
                                          └─(+Onion +Mushroom +Cheese)→ Mushroom Risotto [T7]
                                          └─(+Truffle shavings)→ Truffle Risotto         [T7 — luxury tier]
                                          └─(+Saffron)→ Saffron Risotto                  [T7 — luxury tier]
Rice ─(Brewing Vat +special starter +30-day)→ Sake (rice wine)                            [T7]
Rice + Vinegar + Fish → Sushi                                                              [T8 fishing-integrated]
```

### 6.12 Vegetable Garden Stew (T4 multi-ingredient mid-tier)

```
Carrot + Potato + Beetroot + Onion + Beef + Bowl + Water ─(Cooking Pot 1-day)→ Garden Stew [T4]
  ↑ All optional substitutions — the Cooking Pot accepts any 3+ "vegetable" inputs
   + meat + Bowl + Water as a generic Stew recipe. High saturation, modest hunger.

Interim T1.5 Stew (before Cooking Pot exists) uses the Furnace as a substitute station:
Bowl + 3 vegetables ─(Furnace 200 ticks)→ Stew                                              [T1.5]
T3 Cooking Pot supersedes this Furnace path with a longer-time, higher-quality version.
```

---

## 7. Biome Specialization Map

The world is divided into climate zones; each gates which crops grow naturally and which can be farmed. Greenhouses (T8) eventually break biome gating, but until then trade is the way.

**Note**: Spec 2 §2.5 describes a full multi-biome infrastructure (4×4×4-resolution `biomes: [u16; 64]` per chunk section with palette compression and plugin-injectable biome registry). However, **current worldgen code only emits a single biome in practice** — the registry is wired but biome variation isn't generated yet. The "worldgen biome variety" gap is its own future spec; this section frames what farming asks of it. Crops introduced before active biome variation (T1, T1.5, parts of T3-T4) work everywhere by default; biome gating kicks in from T5 onward when worldgen actually emits varied biomes.

### 7.1 The seven primary biomes

| Biome | Climate | Signature crops | Signature animals | Trade-out specialties |
|---|---|---|---|---|
| **Temperate Plains** | Mild, 4 seasons | Wheat, Barley, Oats, Carrot, Potato, Beetroot, Sugar Beet, Apple, Pear, Plum, Cabbage, Onion, Pea | Cow, Horse, Pig, Chicken, Sheep | Grain, dairy, hardy fruit |
| **Mediterranean Coast** | Warm dry summer, mild wet winter | Tomato, Olive, Grape, Aubergine, Pepper, Citrus, Garlic, all herbs, Fig | Goat, Sheep, Chicken | Wine, olive oil, herbs, citrus |
| **Tropical Jungle** | Hot wet | Cocoa, Coffee, Vanilla, Banana, Coconut, Ginger, Pineapple, Mango, Sugarcane (some) | Chicken (jungle fowl variant?), Pig | Cocoa, coffee, exotic fruits |
| **Cool Highland** | Cool dry, mountainous | Tea, Rye, Buckwheat, hardy herbs (rosemary, thyme on south-slopes) | Goat, Sheep | Tea, specialty cheeses, hops? |
| **Tundra / Boreal** | Cold | Turnip, Swede, hardy berries (Lingonberry), Spruce/Pine for syrup | Reindeer? (post-T6) | Hardy berries, specialty cold-region |
| **Wetland / Paddy** | Hot wet flooded | Rice, Cranberry, Lotus, Water Chestnut | Duck, Goose | Rice, water-grown specialties |
| **Desert / Oasis** | Hot arid | Date Palm, Prickly Pear, drought-hardy herbs | Goat | Dates, exotic desert specialties |

### 7.2 How biome gating works mechanically

Two-level model:

1. **Natural growth** — a crop's `preferred_biomes: Vec<Biome>` field. Outside its preferred biomes, growth is slow (50% rate or refuses entirely).
2. **Greenhouse override** (T8) — Greenhouses provide a "climate zone" microenvironment that overrides natural biome restrictions. Cost: glass + energy + maintenance. Gives the player wealth-tier access to off-biome crops without trade.

### 7.3 Trade implications

Server economy modes (Spec 6 §13) integrate with biome specialization: a player on a Mediterranean server who can grow cocoa via greenhouse has high build-up cost but local supply. A player who trades for cocoa across servers (player-to-player via item-trade-value) has no build-up but trade cost. Both paths viable; both fund the economy.

---

## 8. Value Ladder Structure

Each item has three economic axes from T1.5 onward, plus a fourth and fifth from T7:

1. **`complexity_tier: u8`** — 0 (raw) to 5 (luxury / multi-ingredient masterpiece). Lands T1.5.
2. **`food_value: Option<(hunger: f32, saturation: f32)>`** — what eating it restores. Pre-existing (Spec 5 §6.2).
3. **`trade_value: Option<u64>`** — internal trade-unit value; convertible to sats on Bitcoin-enabled servers. Lands T1.5.
4. **`buff_effect: Option<BuffEffect>`** — temporary status effect on consumption. **Lands T7**, alongside the status-effect runtime that consumes it. **Do NOT add this field at T1.5** — the engine has no status-effect system yet, and adding a dead field invites confusion. T7's foundation spec adds both the field and its consumer in one slice.
5. **`superfood: bool` + `diet_score: i8`** — drives the dynamic hunger-capacity system (§8.4). **Lands T7**, alongside the dynamic-capacity runtime. The flag is data-only until the consumer ships; mark it from T1.5 onward as informational.

### 8.1 Complexity tiers explained

| Tier | Definition | Examples | Hunger range | Sat range | Trade-units | Buff? |
|:---:|---|---|:---:|:---:|:---:|:---:|
| **0** | Raw — grown, harvested, mob-dropped, no processing | Wheat, Carrot, Raw Beef, Egg, MilkBucket | 0-2 | 0-1 | 1-3 | No |
| **1** | Single-process — one workstation step | Flour, Sugar, Cooked Beef, Baked Potato, Cream, Beer (young), Apple Juice | 3-6 | 2-4 | 4-10 | No |
| **2** | Two-process — chained workstations | Bread (Oven path), Butter, Cider, Cheese (basic), Tomato Sauce | 6-9 | 4-7 | 12-25 | No |
| **3** | Multi-input — recipe combines 3+ tier 0/1 inputs | Cake, Berry Pie, Pumpkin Pie, Stew, Ratatouille, Risotto (basic) | 10-14 | 7-11 | 30-60 | Rare (mild) |
| **4** | Refined — premium ingredients + skill recipe | Wine (aged), Cheddar, Pizza, Lasagne, Hot Chocolate | 12-16 | 10-14 | 75-150 | Common (mild) |
| **5** | Luxury — rare ingredients + long aging + multi-step | Truffle Risotto, Parmesan (90-day), Vintage Wine, Saffron dishes, Aged Brandy | 18-20 | 18-20 | 200-1000 | Strong, multi-effect |

The ranges overlap intentionally — a tier-2 item can be more filling than a low-tier-3 if the recipe favours saturation over breadth.

### 8.2 Trade-value scaling intuition

The table values above aren't computed — they're hand-tuned. But the shape of the curve should follow three intuitions:

- **Depth dominates**: a tier-3 item should be ~3× a tier-2 item, not 1.5×. Reward chained processing steeply.
- **Rarity multiplies**: common ingredients get a 1× multiplier; saffron / truffle / vanilla can go up to ~5×.
- **Time-investment matters**: a 3-day cheese should be ~3× a 1-day butter, even at the same complexity tier. Patient recipes earn more.

For implementer reference, an item's trade value tends to land near `base_tier_value × rarity × time_factor`, where `base_tier_value` is roughly the midpoint of the tier's range from the table (3, 7, 18, 45, 110, 500), `rarity` is 1–5×, and `time_factor` is 1× for tick-driven, up to ~3× for week-aged. This is illustrative, not auto-computed — playtest tuning overrides any formula.

### 8.3 Buff effects (status-effect system lands T7; recipe-side designs span T3–T8)

The **status-effect runtime** (the consumer of `buff_effect`) lands at T7. The buff-carrying foods themselves span T4–T8 by recipe earliest-tier — the table below assigns buffs to recipes that already exist at their listed tier; buffs only become *active* once T7 ships. Until then, these items work as plain food (their food/trade values apply; buff_effect is a no-op).

| Food | Buff | Duration | Strength |
|---|---|---|---|
| Hot Chocolate | Warming (cold biome resistance) | 3 min | I |
| Coffee | Alertness (sprint stamina drain ÷ 2) | 5 min | I |
| Espresso | Alertness | 5 min | II |
| Garlic Bread | Vampire/zombie aversion (mob aggro -20%) | 2 min | I |
| Honey Tea | Healing (regen +1) | 2 min | I |
| Chilli Con Carne | Fire resistance | 1 min | I |
| Truffle Risotto | Luck (drop-rate +10%) | 5 min | II |
| Saffron Risotto | Luck | 5 min | III |
| Aged Wine | Strength | 3 min | I |
| Vintage Wine | Strength | 3 min | II |
| Parmesan-topped dish | Saturation extended | passive | — |
| Honey | Healing | 1 min | I |
| Maple Syrup (pure) | Quick energy (sprint speed +10%) | 1 min | I |
| Sushi (T8) | Water breathing | 3 min | I |
| Mint Tea | Cleanse status effects (one-shot) | instant | — |

Designed so buffs are useful but not game-breaking. No "permanent buff" tier — every effect is timed.

### 8.4 Superfoods & dynamic health + hunger capacity

A subset of nutrient-dense foods carry the **Superfood** flag, which feeds into a **dynamic capacity system** that replaces the fixed Minecraft-style health and hunger bars. Both bars scale together — eat well and move, both grow; eat poorly and sit still, both shrink. Lands T7 (alongside the status-effect runtime — same delivery slice, since both consume per-item flags laid down from T1.5 onward).

**Capacity model (applies to both health and hunger bars in lockstep):**

- **Starting capacity**: 8 slots on each bar (smaller than Minecraft's fixed 10 — leaves headroom to grow).
- **Range**: 4 (minimum) to 12 (maximum) on each bar.
- **Drivers** — capacity drifts up or down based on two parallel rolling-window inputs:
  - **Diet** — aggregated `diet_score` from food eaten in the last ~24 real-minutes. Superfoods score high (+3); plain cooked food +1; raw veg 0; sugar-heavy / mono-diet -1 to -2.
  - **Movement** — `activity_score` from sprint-ticks, blocks-mined, combat-hits in the same window. Sedentary play decays this; high activity grows it.
- **Drift rate** (illustrative, playtest-tuned): ~1 slot per real-minute, so the bars don't visibly flicker. Capacity growth above the current fill doesn't instantly refill — you still have to eat / regen to claim the new space.
- **Lockstep vs split?** Open question (see below): bars could move together (one capacity number drives both) or independently (eat for hunger, exercise for health). Default proposal: **single capacity number** drives both — keeps the mental model simple for Axolittle's age group. Possible split as a T8+ depth lever.

**Superfood roster** (canonical list — code data file owns the authoritative `superfood: bool` flag; §3 catalogue rows do not duplicate the tag, this table is the source of truth in docs):

| Category | Superfoods |
|---|---|
| Leafy greens | Spinach, Kale, Watercress, Pak Choi |
| Brassicas | Broccoli |
| Berries | Blueberry, Blackcurrant, Lingonberry, Elderberry, Cranberry |
| Nuts | Walnut, Almond, Pecan |
| Legumes / pseudo-grain | Lentils, Chickpeas, Soybeans, Peanut, Quinoa |
| Tree fruit | Avocado, Pomegranate, Fig, Olive (via oil) |
| Specialty | Honey |
| Fish (T8 — separate spec) | Salmon, Sardines, Mackerel (omega-3 leaders) |

**Why this matters for the economy:**

- **Demand-side**: superfoods carry a structural premium beyond their hunger refill — they expand both your health *and* hunger ceiling, not just top them up. Drives sustained demand for leafy greens, berries, nuts, and the T5+ tropical specialties.
- **Supply-side**: low-tier filler foods keep their hunger-refill floor but can't replace superfood demand.
- **Trade**: cross-biome trade gets a kick — temperate-plains players want jungle/tropical superfoods (avocado, mango, banana, cashew) to keep their bars in the upper half.
- **Mid-game pacing**: T1-T4 players run with smaller bars (~6-8); T5+ players with mature trade routes hit 10-12 reliably. Bar size is itself a progression indicator — a wandering trader with 12/12 bars *looks* established.
- **Combat survivability**: PvE servers see a structural advantage for farmer-fighters — your 12-heart farmer survives the cave dive that one-shots the 4-heart hermit.

**Open design questions** (Axolittle's calls before T7 ships):

1. Should bars **shrink visibly** on a mono-diet, or only **fail to grow**? (Punitive vs aspirational framing.)
2. What counts as "junk" on the negative end? Cake every meal? Pure-sugar diet?
3. Does sleep / rest affect drift (e.g. recovery vs decay)?
4. How fast should bars drift — 1 slot/min, 1 slot/3-min? Faster = more responsive but more anxious.
5. Should activity-score include creative-mode flight? (Probably no — it's a survival mechanic.)
6. Single capacity (both bars together) or split (eat for hunger, exercise for health)? Default is single; split is a T8+ depth lever.
7. Below-minimum penalty: if a player keeps both bars at 4/4, are there visible disadvantages (slower mining? lower jump?) or is the small bar itself the penalty?

---

## 9. Economy Mechanisms

What makes the economy *strong* — not just present?

### 9.1 Demand drivers

- **Hunger replenishment** — players always need food. The baseline demand floor.
- **Buff foods** — for PvE-heavy servers, buff foods drive combat-prep demand.
- **Crafting inputs** — some items (Cheese for Loaded Baked Potato, Hops for Beer) drive demand for raw inputs from outside the foodchain.
- **Trade-value** — on Bitcoin-enabled servers, items convert to sats; demand from sats-seekers.
- **Vendor blocks (T8)** — server-operator placed NPC traders consume specific items for sats. Out of scope for vision; framing slot.

### 9.2 Supply constraints

- **Time-to-grow** — caps farming throughput. Aging Rack and Cellar cap processed-good throughput further.
- **Biome specialization** — limits who can grow what locally.
- **Tool durability** — Hoes wear out (T1 already); higher-tier farming tools (Plough at T2, etc.) wear too.
- **Land area** — players have to allocate space; can't grow infinite parallel.
- **Animal product cooldowns** — milking + egg-laying cooldowns prevent infinite-rate extraction.
- **Workstation count** — each station processes one recipe at a time; scaling production requires building more stations (more materials, more space).

### 9.3 Trade enablers

- **Item trade-value** — annotation system (T1.5 Phase 11) sets the trade backbone.
- **Biome trade** — different crops in different biomes mechanically force trade (T5+).
- **Player-to-player trade UI** — out of scope for T1-T5; a future spec.
- **Server marketplace blocks** — out of scope; future spec.
- **Bitcoin/sats conversion** — Spec 6 §13 (Bitcoin-enabled servers); parent-controlled.

### 9.4 Sinks (where value goes to die)

A healthy economy needs sinks — outflows that prevent infinite accumulation:

- **Food consumption** — every eaten item is removed.
- **Tool durability** — every farming action chips at hoes/buckets/scythes.
- **Spoilage** (optional, T7+) — if added, perishable items expire.
- **Workstation fuel** — Coal/wood burns away. Furnaces and Ovens are perpetual sinks.
- **Vendor sell-back** (T8) — convert items to sats, removing the items.

Without sinks, prices crash. Without sources, prices spike. Tier-by-tier balancing is a Phase-13-equivalent ongoing activity.

---

## 10. Cross-Cutting Design Choices

### 10.1 Should spoilage exist?

**Recommended: No, until T7+.** Spoilage adds anxiety + grind without proportional fun. *If* added at T7, only for high-value premium items (cheese cave outputs, prepared meats) — never for staple grains/vegetables. Spoiled items become Compost (T7 Composter input). This makes spoilage a low-tier-value-recovery loop, not a punishment.

### 10.2 Should quality grades exist?

**Recommended: Yes at T7.** Each harvest rolls a quality grade (common 70% / fine 25% / exquisite 5%) influenced by:
- Hoe tier (Satori Hoe → +1% exquisite chance)
- Biome match (preferred biome → +5% fine, +2% exquisite)
- Skill / playstyle (player Farming-XP if XP system exists by then)

Quality grade multiplies trade-value (×1, ×1.5, ×3). Doesn't affect food-value (still feeds you the same). Adds collectable + trade-arbitrage depth.

### 10.3 Should seasons exist?

**Recommended: Optional, T8 polish.** Seasonal cycles (spring/summer/autumn/winter) add depth but require worldgen + lighting changes. If added, some crops are seasonal (strawberry only in summer; pumpkin only in autumn) and others perennial.

### 10.4 Should fertilizer / compost exist?

**Recommended: Yes at T7.** Composter (T7 workstation) takes food waste → Compost. Compost applied to tilled soil → faster growth (matches Minecraft bonemeal but more thematic). Closes the waste-loop. Compost can also be sold/traded.

### 10.5 Tool tiers for farming-specific tools

| Tool | Tiers | Notes |
|---|---|---|
| Hoe | Wood, Stone, Iron, Diamond, Satori | T1 — durability + till speed scale |
| Bucket | Iron only | T1.5 — Minecraft baseline |
| Plough | TBD — single iron tier? Per-tier? | T2 design open question |
| Watering Can | Wood, Iron | T7? — alternative to natural water-adjacency growth boost |
| Sickle / Scythe | Stone, Iron, Diamond, Satori | T7? — area-harvest tool for wheat/grain blocks |
| Pruning Shears | Iron | T3 — orchard maintenance |
| Maple Tap | Wood, Iron | T6 — single-use? Durable? |
| Beekeeper's Smoker | Iron | T6 — pacifies bees for honey harvest |
| Fishing Rod | Wood, Stone, Iron, Diamond | T8 — separate spec |

### 10.6 Recipe-book / discovery UI

Spec 5 §4.6 calls for a recipe book. **Recommended delivery: T4** — once the recipe count exceeds ~50 across multiple workstations, the recipe-book UI becomes essential. Before T4 it's `/help recipes` console command + workstation-UI auto-suggestion.

### 10.7 Oils as machinery lubricant

Pressed oils have a **second role** beyond cooking: applied to powered machinery and carts, they reduce wear and increase throughput speed. This makes the Oil Press (T5) a *production* workstation, not just a *food* one — anchoring a structural demand floor for olive, peanut, sunflower, rapeseed, and linseed.

**Eligible oils:**

| Oil | Source | Crop tier | Lubricant grade |
|---|---|:---:|:---:|
| Olive Oil | Olive (Mediterranean tree) | T5 | Premium (best speed multiplier) |
| Peanut Oil | Peanut (warm temperate legume) | T5 | Good |
| Sunflower Oil | Sunflower (warm temperate, §3.15) | T7 | Good |
| Rapeseed Oil | Rapeseed / Canola (§3.15) | T7 | Standard (commodity) |
| Linseed Oil | Flax seed (§3.15) | T8 | Wood-finishing specialty (carts only, not industrial) |
| Cashew Oil | Cashew nut | T7 | Niche (tropical-server option) |

**Mechanic** (illustrative, playtest-tuned):

- **Carts (T2+)** — every cart has a hidden `lubrication: f32` 0.0-1.0. Drains slowly with movement. Applying oil refills it. Effects:
  - Above 0.5 → +20% top speed, normal wear.
  - Below 0.2 → -30% top speed, +50% wear (parts wear out faster).
  - Empty → cart still works but slow and noisy, accelerated breakdown.
- **Industrial workstations (T8)** — Industrial Mill, Power Oven, etc. have an internal `lubrication_level`. Above threshold → +15% processing speed. Below threshold → -20%. Refilled via oil-slot insertion.
- **Tools (T2+)** — applying oil to a Plough / Hoe / Scythe restores a chunk of durability (cheaper than re-crafting, more than a single repair).

**Oil grade differences** (premium vs standard):

- Olive Oil gives the largest speed/longevity bonus per unit; high-end servers see olive-belt regions corner the lubricant market.
- Rapeseed / Sunflower are the commodity oils — cheap, plentiful, "good enough" for most use.
- Linseed Oil doubles as the wood-cart finish — applied to a cart's body it adds a slow water-damage resistance buff (carts in rain degrade less).

**Why this matters:**

- Gives **oils** a non-food role, anchoring demand for the Oil Press chain even on servers where olive recipes haven't taken off culturally.
- Creates a recurring trade for **Mediterranean (olive) and warm-temperate (peanut/sunflower/rapeseed)** crops on industrial-tier servers.
- Adds a depth lever to T2 transport — *neglected carts are slow carts*; players who farm oil have a tangible edge in mining-haul throughput.
- Pairs with the **Genesis-Block / Satori** late-game progression — top-tier diamond/Satori machinery especially benefits from premium oil.

**Delivery tiers:**

- T2 introduces the cart `lubrication` field (basic; works without oil but with a small wear-rate penalty if empty).
- T5 introduces the Oil Press + Olive Oil and Peanut Oil — first oils actually available.
- T7 expands to Sunflower / Rapeseed / Cashew oils.
- T8 introduces the industrial-workstation `lubrication_level` system.

Until T5 ships, carts run without lubricant penalties (or with a flat default fill) — the system is dormant. T2 lays the data-field so retrofitting later is no migration.

---

## 11. Open Design Questions (for Axolittle)

Real TBDs that need his calls before the relevant tier ships:

### Affecting T2 (already from Tier 1 spec)
1. Plough recipe tier — single iron-tier, or per-tier ladder?
2. Fence-autonomy edge cases — incomplete enclosure behaviour?
3. Manual plough usable without animal?

### Affecting T3 (Orchards + Drinks)
4. Alcohol in a kid-target game — is "beer", "wine", "cider" appropriate, or do we re-frame as "fizzy apple drink", "grape juice", etc.? **Recommendation:** keep the real names; alcohol items are flavour/recipe-only (no drunkenness mechanic); parents can disable on per-server basis if uncomfortable. **Asks Axolittle + Staxolottle.**
5. Tree growth timescale — minutes? Hours? Days? An apple tree taking 3 in-game days to mature would matter for orchard planning.

### Affecting T4 (Preservation + Variety)
6. How many crop variants should drop the same generic "Vegetable" / "Stew Ingredient" tag vs being distinct items? (Trade-off: variety vs registry bloat.)

### Affecting T5 (Biome Specialization)
7. How "strict" should biome gating be? Soft (slower growth off-biome) or hard (refuses to grow)? Recommendation: soft for kid-friendliness — never *fully* blocks, just makes off-biome growing inefficient enough that trade is preferable.
8. Should the player be able to introduce crops to non-native biomes via "seed prestige" (heirloom adaptation)? Adds depth; complicates.

### Affecting T6 (Bees + Maple)
9. Bee aggression — Minecraft has angry bees on hive disturbance. Adjust for kid-target?
10. Maple tapping — passive or interactive? Daily check-in or fully passive?

### Affecting T7 (Buffs + Mastery + Quality)
11. Quality grade RNG vs skill-based — should quality be lucky-roll, or earnable via repeated practice (Farming-XP)?
12. Buff effect strength — should buffs be game-changing (PvE relevant) or just flavour? Pitch is "useful but not game-breaking" — Axolittle's call.

### Affecting T8 (Industrial)
13. Power source for industrial workstations — water wheel? Steam? Wind? All three? Or simplify to "Mechanical Power" as a generic? Has cross-cutting implications for redstone-equivalent.
14. Greenhouse cost — should it be expensive enough to feel like an end-game build, or accessible enough that mid-game players can specialize?

### Cross-cutting
15. Spoilage — yes/no? (My recommendation: no until T7, then only for premium.)
16. Hunger drain rate — current Spec 5 §6.2 says ~1 hunger/4-seconds at rest. Does the abundance of T4+ food make this trivial? Recalibrate at each tier?
17. Player-to-player trade UI — when in the roadmap? Out of scope here but the trade-value annotation is laid in early; the UI itself is a future spec.

---

## 12. Foundation Spec Cross-References

Where each tier's implementation contract lives (and when each gets drafted):

| Tier | Foundation Spec | Status |
|:---:|---|---|
| T1 | `docs/foundations/2026-05-14-farming-system.md` | READY TO BUILD |
| T1.5 | `docs/foundations/2026-05-14-farming-tier-1.5-processed-economy.md` | READY TO BUILD (after T1) |
| T2 | TBD — sketch lives at bottom of T1 spec | Draft after T1 ships + Axolittle playtest |
| T3 | TBD | Draft after T2 ships + playtest |
| T4 | TBD | Draft after T3 ships |
| T5 | TBD | Draft after T4 ships + worldgen-biome spec exists |
| T6 | TBD | Draft after T5 ships |
| T7 | TBD | Draft after T6 ships |
| T8 | TBD | Draft after T7 ships + power-system spec exists |

**Each tier spec derives from this master design.** When designing a tier spec:
1. Pull from the master crop catalogue (§3) for what crops to include.
2. Pull from the master workstation catalogue (§5) for which stations land.
3. Pull from the master recipe chains (§6) for what to recipe-out.
4. Honor the biome map (§7) for any crops introduced.
5. Honor the value ladder (§8) for `complexity_tier` and `trade_value` assignments.
6. Update this master document if a tier's playtest reveals a design call that contradicts the vision — the master is *living*, not frozen.

---

## 13. Visual / UX Considerations

### 13.1 Crop registry pressure

85 crops × 4-5 growth stages each = ~400 crop block IDs. Plus ~90 item textures for the harvested forms. Plus ~150-200 finished-food item textures. **Total texture-gen burden: ~650 unique sprites.**

This is manageable for procedural pixel art (current AxeNStax texture-gen approach) but **needs an art-direction style guide** so all crops look like they belong in the same world. **Recommendation: a T1 polish phase or T1.5 art audit** that locks the visual conventions (palette per crop family, growth-stage progression style, item-icon framing).

### 13.2 Inventory pressure

A player with a mature farm has dozens of stack types. Current Spec 5 §3.1 inventory is 36 slots + 9 hotbar. **Long-run consideration:** chest interaction (block-entity, already laid in by T1.5's framework) becomes the storage solution. Sorting / search UI is a future polish.

### 13.3 Workstation block count

21 workstation types is a lot. **Pack them into a "Farming/Cooking" creative-inventory category** to keep the creative inventory navigable. Same idea for crop blocks — group under "Crops" subcategory.

### 13.4 Recipe density

By T7 there are 100+ recipes across all workstations. **The recipe-book UI (Spec 5 §4.6, T4 delivery) becomes essential** — players can't memorise 100+ recipes. Categorise by workstation, then by complexity tier.

---

## 14. Cross-Game Lift Notes

The following pieces are explicitly designed for lift into other AxeNStax-internal games per shared infra strategy:

- **Workstation framework** — block-entity + slot + tick + recipe-table pattern. Lifts directly.
- **Aging Rack in-game-day timer model** — generic slow-time mechanic. Lifts to any game wanting "set and come back later" depth.
- **Trade-value annotation** — generic item-economy primitive. Same `Item::trade_value()` method works in any inventory game.
- **Complexity-tier system** — same.
- **Breeding system** (planned for T2) — generic mob-breeding pattern. Lifts.
- **Biome specialization data** — biome enum + `preferred_biomes` field on crop data. Lifts to any biome-aware game.
- **Quality grade RNG** (T7) — generic harvest-quality pattern. Lifts.
- **Buff system** (T7) — generic temporary-status-effect framework. Lifts.

What stays AxeNStax-specific:
- The actual crop / animal / recipe lists (data files)
- Recipe names ("Cake", "Truffle Risotto", etc.)
- Biome names and crop-biome mappings
- Buff effect names and durations

Keep this split disciplined: framework code in `engine/src/`, data tables in `engine/data/` (or analogous). Future games override the data, reuse the framework.

---

## 15. Memory-rule check

- **signet boundary**: N/A.
- **shared infra strategy**: ✓ — explicitly designed for lift; §14 documents the split.
- **pretest check**: not implementation, no code claims to verify.
- **axenstax has farming**: ✓ — this doc is the long-run extension of the design captured there.
- **uk english naming**: ✓ — names checked against UK conventions throughout (aubergine, courgette, coriander, beetroot ≠ sugar beet).
- **alpha launch posture**: ✓ — T1 / T1.5 / T2 are post-alpha priority; this master is the long-run vision spanning post-alpha through long-run content.
- **bitcoin parent controlled**: ✓ — Bitcoin layer is server-operator opt-in; default economy is internal-trade-units-only.
- **proof of play is proof of work**: ✓ — cooking explicitly does NOT invoke proof-of-play; trade-value is the Bitcoin hook, not hash-on-bake.

---

## 16. Glossary

- **Tier** — A delivery phase (T1 … T8). Each tier ships an internally coherent slice.
- **Complexity tier** — The 0-5 value-ladder position of a finished item (not the same as delivery tier, despite both using "tier").
- **Workstation** — A block-entity processing station (Furnace, Mill, Oven, etc.).
- **Workstation framework** — The generic block-entity + slot + tick + recipe pattern delivered in T1.5 Phase 2.
- **Aging Rack** — Workstation with in-game-day timer (vs tick timer) for slow recipes.
- **Cellar** — T7's premium cheese/wine aging facility. In-game-week timer. Climate-controlled.
- **Value ladder** — The complexity-tier 0-5 progression with associated food/trade values.
- **Trade-value** — Abstract internal currency on items. Always present. Convertible to sats on Bitcoin-enabled servers per Spec 6 §13.
- **Buff** — Temporary status effect granted by tier-4+ foods. Always timed, never permanent.
- **Heirloom seed** — T7 rare/specialty variant of a common crop (higher value, more demanding to grow).
- **Quality grade** — T7 harvest randomisation (common / fine / exquisite) multiplying trade-value.
- **Biome gating** — Whether a crop grows efficiently in a given biome. T5+.
- **Greenhouse** — T8 climate-controlled grow space that overrides natural biome gating.
